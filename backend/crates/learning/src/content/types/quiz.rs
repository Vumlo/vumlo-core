//! The `quiz` type (ADR-0050): the config, the answers, the scoring and the student's view.

use std::collections::{BTreeMap, BTreeSet};

use api_types::ProfileName;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::types::Decimal;

use super::{
    AutoGrade, Problems, Stage, ValidContent, answer_path, answers_of, document_problem,
    parse_number, shape_problem, valid_id,
};
use crate::content::document::{DocumentErrors, EffectiveProfiles, validate_document};
use crate::content::model::{MaxPoints, ModelError};

const MAX_QUESTIONS: usize = 100;
const MAX_OPTIONS: usize = 10;
const MAX_ACCEPTED: usize = 20;
const MAX_CONFIG_BYTES: usize = 2 * 1024 * 1024;
const MAX_ANSWER_CHARS: usize = 500;
const MAX_NOTES_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuestionType {
    SingleChoice,
    MultipleChoice,
    Number,
    ShortText,
    LongText,
}

impl QuestionType {
    fn as_str(self) -> &'static str {
        match self {
            Self::SingleChoice => "single_choice",
            Self::MultipleChoice => "multiple_choice",
            Self::Number => "number",
            Self::ShortText => "short_text",
            Self::LongText => "long_text",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    v: u8,
    questions: Vec<RawQuestion>,
    // Checked as a bool and kept in the config; only the client shuffles.
    #[serde(default)]
    #[allow(dead_code)]
    shuffle_questions: bool,
    #[serde(default)]
    show_correct: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQuestion {
    id: String,
    #[serde(rename = "type")]
    kind: QuestionType,
    points: String,
    prompt: Value,
    explanation: Option<Value>,
    grading_notes: Option<String>,
    options: Option<Vec<RawOption>>,
    correct: Option<Vec<String>>,
    shuffle_options: Option<bool>,
    partial_credit: Option<bool>,
    answer: Option<String>,
    tolerance: Option<String>,
    accepted: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    id: String,
    content: Value,
}

/// A question after it passed the checks, with the numbers decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Question {
    id: String,
    kind: QuestionType,
    points: Decimal,
    options: Vec<String>,
    correct: Vec<String>,
    partial_credit: bool,
    answer: Option<Decimal>,
    tolerance: Decimal,
    /// Trimmed and lowercased.
    accepted: Vec<String>,
}

struct Quiz {
    questions: Vec<Question>,
    show_correct: bool,
}

/// Which optional fields each question type may carry.
fn allowed_fields(kind: QuestionType) -> &'static [&'static str] {
    match kind {
        QuestionType::SingleChoice => &["options", "correct", "shuffle_options"],
        QuestionType::MultipleChoice => {
            &["options", "correct", "shuffle_options", "partial_credit"]
        }
        QuestionType::Number => &["answer", "tolerance"],
        QuestionType::ShortText => &["accepted"],
        QuestionType::LongText => &[],
    }
}

/// Parses and checks a config. The documents inside are validated only when `profiles` is given:
/// scoring reads a config that was validated when it was written.
fn parse(
    config: &Map<String, Value>,
    profiles: Option<&EffectiveProfiles>,
) -> Result<Quiz, ModelError> {
    let bad = |message: String| ModelError::Config(message);
    let size = serde_json::to_vec(config).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_CONFIG_BYTES {
        return Err(bad("the quiz config is larger than 2 MiB".to_owned()));
    }
    let raw: RawConfig = serde_json::from_value(Value::Object(config.clone()))
        .map_err(|error| shape_problem("quiz", &error))?;
    if raw.v != 1 {
        return Err(bad("`v` must be 1".to_owned()));
    }
    if !(1..=MAX_QUESTIONS).contains(&raw.questions.len()) {
        return Err(bad(format!(
            "`questions` must have 1 to {MAX_QUESTIONS} questions"
        )));
    }
    let mut seen = BTreeSet::new();
    let mut questions = Vec::with_capacity(raw.questions.len());
    for (index, q) in raw.questions.into_iter().enumerate() {
        let path = format!("questions[{index}]");
        if !valid_id(&q.id) || !seen.insert(q.id.clone()) {
            return Err(bad(format!(
                "`{path}.id` must be 1 to 64 letters, digits, `_` or `-`, and unique"
            )));
        }
        let points = MaxPoints::parse(&q.points)
            .map_err(|_| {
                bad(format!(
                    "`{path}.points` must be a number above 0 and at most 9999.99, with at most two decimals"
                ))
            })?
            .value();
        if let Some(profiles) = profiles {
            validate_document(&q.prompt, ProfileName::Question, profiles)
                .map_err(|errors| document_problem(&format!("{path}.prompt"), &errors))?;
            if let Some(explanation) = &q.explanation {
                validate_document(explanation, ProfileName::Question, profiles)
                    .map_err(|errors| document_problem(&format!("{path}.explanation"), &errors))?;
            }
        }
        if q.grading_notes
            .as_ref()
            .is_some_and(|notes| notes.chars().count() > MAX_NOTES_CHARS)
        {
            return Err(bad(format!(
                "`{path}.grading_notes` is longer than {MAX_NOTES_CHARS} characters"
            )));
        }
        let present = [
            ("options", q.options.is_some()),
            ("correct", q.correct.is_some()),
            ("shuffle_options", q.shuffle_options.is_some()),
            ("partial_credit", q.partial_credit.is_some()),
            ("answer", q.answer.is_some()),
            ("tolerance", q.tolerance.is_some()),
            ("accepted", q.accepted.is_some()),
        ];
        let allowed = allowed_fields(q.kind);
        if let Some((name, _)) = present
            .iter()
            .find(|(name, given)| *given && !allowed.contains(name))
        {
            return Err(bad(format!(
                "`{path}.{name}` does not belong to a {} question",
                q.kind.as_str()
            )));
        }
        let mut question = Question {
            id: q.id,
            kind: q.kind,
            points,
            options: Vec::new(),
            correct: Vec::new(),
            partial_credit: q.partial_credit.unwrap_or(false),
            answer: None,
            tolerance: Decimal::ZERO,
            accepted: Vec::new(),
        };
        match q.kind {
            QuestionType::SingleChoice | QuestionType::MultipleChoice => {
                let options = q
                    .options
                    .ok_or_else(|| bad(format!("`{path}.options` is required")))?;
                if !(2..=MAX_OPTIONS).contains(&options.len()) {
                    return Err(bad(format!(
                        "`{path}.options` must have 2 to {MAX_OPTIONS} options"
                    )));
                }
                let mut ids = BTreeSet::new();
                for (n, option) in options.iter().enumerate() {
                    if !valid_id(&option.id) || !ids.insert(option.id.clone()) {
                        return Err(bad(format!(
                            "`{path}.options[{n}].id` must be 1 to 64 letters, digits, `_` or `-`, and unique in the question"
                        )));
                    }
                    if let Some(profiles) = profiles {
                        validate_document(&option.content, ProfileName::Option, profiles).map_err(
                            |errors| {
                                document_problem(&format!("{path}.options[{n}].content"), &errors)
                            },
                        )?;
                    }
                }
                let correct = q
                    .correct
                    .ok_or_else(|| bad(format!("`{path}.correct` is required")))?;
                let distinct: BTreeSet<&String> = correct.iter().collect();
                let count_ok = if q.kind == QuestionType::SingleChoice {
                    correct.len() == 1
                } else {
                    !correct.is_empty()
                };
                if !count_ok
                    || distinct.len() != correct.len()
                    || correct.iter().any(|id| !ids.contains(id))
                {
                    return Err(bad(format!(
                        "`{path}.correct` must name {} of the question's options, once each",
                        if q.kind == QuestionType::SingleChoice {
                            "exactly one"
                        } else {
                            "at least one"
                        }
                    )));
                }
                question.options = options.into_iter().map(|option| option.id).collect();
                question.correct = correct;
            }
            QuestionType::Number => {
                let answer = q
                    .answer
                    .ok_or_else(|| bad(format!("`{path}.answer` is required")))?;
                question.answer = Some(
                    parse_number(&answer)
                        .ok_or_else(|| bad(format!("`{path}.answer` must be a number")))?,
                );
                if let Some(tolerance) = q.tolerance {
                    question.tolerance = parse_number(&tolerance)
                        .filter(|value| *value >= Decimal::ZERO)
                        .ok_or_else(|| {
                            bad(format!("`{path}.tolerance` must be a number of at least 0"))
                        })?;
                }
            }
            QuestionType::ShortText => {
                let accepted = q
                    .accepted
                    .ok_or_else(|| bad(format!("`{path}.accepted` is required")))?;
                let trimmed: Vec<String> = accepted
                    .iter()
                    .map(|text| text.trim().to_lowercase())
                    .collect();
                if !(1..=MAX_ACCEPTED).contains(&trimmed.len())
                    || trimmed
                        .iter()
                        .any(|text| text.is_empty() || text.chars().count() > 200)
                {
                    return Err(bad(format!(
                        "`{path}.accepted` must have 1 to {MAX_ACCEPTED} answers of 1 to 200 characters"
                    )));
                }
                question.accepted = trimmed;
            }
            QuestionType::LongText => {}
        }
        questions.push(question);
    }
    Ok(Quiz {
        questions,
        show_correct: raw.show_correct,
    })
}

/// Checks a quiz config (ADR-0050 points 1 and 2).
pub fn validate_config(
    config: &Map<String, Value>,
    profiles: &EffectiveProfiles,
) -> Result<(), ModelError> {
    parse(config, Some(profiles)).map(|_| ())
}

/// The sum of the points of the questions, which `max_points` must equal.
pub fn points_total(config: &Map<String, Value>) -> Option<Decimal> {
    parse(config, None)
        .ok()
        .map(|quiz| quiz.questions.iter().map(|q| q.points).sum())
}

/// The config without the answer key and the notes for staff (ADR-0050 point 6).
pub fn student_config(config: &Map<String, Value>) -> Map<String, Value> {
    let mut config = config.clone();
    if let Some(Value::Array(questions)) = config.get_mut("questions") {
        for question in questions {
            if let Value::Object(question) = question {
                for key in [
                    "correct",
                    "answer",
                    "tolerance",
                    "accepted",
                    "explanation",
                    "partial_credit",
                    "grading_notes",
                ] {
                    question.remove(key);
                }
            }
        }
    }
    config
}

/// The part of a config that cannot change once a quiz has been submitted (ADR-0050 point 7).
pub fn structure(config: &Map<String, Value>) -> Option<Value> {
    let quiz = parse(config, None).ok()?;
    let sorted = |ids: &[String]| {
        let mut ids = ids.to_vec();
        ids.sort();
        ids
    };
    Some(Value::Array(
        quiz.questions
            .iter()
            .map(|q| {
                json!({
                    "id": q.id,
                    "type": q.kind.as_str(),
                    "points": q.points.normalize().to_string(),
                    "options": sorted(&q.options),
                    "correct": sorted(&q.correct),
                    "partial_credit": q.partial_credit,
                    "answer": q.answer.map(|answer| answer.normalize().to_string()),
                    "tolerance": q.tolerance.normalize().to_string(),
                    "accepted": sorted(&q.accepted),
                })
            })
            .collect(),
    ))
}

/// A list of option ids from an answer value, or `None` when it is not one.
fn choice_list(value: &Value) -> Option<Vec<&str>> {
    value
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
}

/// Checks the answers of a draft or an attempt and returns the form that is stored: unanswered
/// questions left out, a long answer in its canonical form (ADR-0050 point 3).
pub fn validate_submission(
    config: &Map<String, Value>,
    content: &Value,
    _stage: Stage,
    profiles: &EffectiveProfiles,
) -> Result<ValidContent, DocumentErrors> {
    let quiz =
        parse(config, None).map_err(|_| DocumentErrors::one("", "the quiz is misconfigured"))?;
    let answers = answers_of(content)?;
    let mut problems = Problems::default();
    let mut kept = Map::new();
    for (id, value) in answers {
        let path = answer_path(id);
        let Some(question) = quiz.questions.iter().find(|q| &q.id == id) else {
            problems.add(path, "is not a question of this quiz");
            continue;
        };
        match question.kind {
            QuestionType::SingleChoice | QuestionType::MultipleChoice => {
                let Some(picked) = choice_list(value) else {
                    problems.add(path, "must be a list of option ids");
                    continue;
                };
                let distinct: BTreeSet<&&str> = picked.iter().collect();
                if distinct.len() != picked.len()
                    || picked
                        .iter()
                        .any(|id| !question.options.iter().any(|o| o == id))
                    || (question.kind == QuestionType::SingleChoice && picked.len() > 1)
                {
                    problems.add(
                        path,
                        "must name options of the question, once each, and one for a single choice",
                    );
                    continue;
                }
                if !picked.is_empty() {
                    kept.insert(id.clone(), value.clone());
                }
            }
            QuestionType::Number => match value.as_str() {
                Some(text) if text.trim().is_empty() => {}
                Some(text) if parse_number(text).is_some() => {
                    kept.insert(id.clone(), Value::String(text.trim().to_owned()));
                }
                _ => problems.add(path, "must be a number"),
            },
            QuestionType::ShortText => match value.as_str() {
                Some(text) if text.chars().count() > MAX_ANSWER_CHARS => {
                    problems.add(path, "is longer than 500 characters");
                }
                Some(text) if text.trim().is_empty() => {}
                Some(text) => {
                    kept.insert(id.clone(), Value::String(text.trim().to_owned()));
                }
                None => problems.add(path, "must be text"),
            },
            QuestionType::LongText => {
                match validate_document(value, ProfileName::Student, profiles) {
                    Ok(document) => {
                        if !document.is_blank() {
                            kept.insert(id.clone(), Value::Object(document.as_body().clone()));
                        }
                    }
                    Err(errors) => {
                        for error in errors.iter() {
                            problems.add(
                                if error.path.is_empty() {
                                    path.clone()
                                } else {
                                    format!("{path}.{}", error.path)
                                },
                                error.message,
                            );
                        }
                    }
                }
            }
        }
    }
    problems.into_result(|| {
        let blank = kept.is_empty();
        let mut body = Map::new();
        body.insert("answers".to_owned(), Value::Object(kept));
        ValidContent::new(body, blank)
    })
}

/// The score of one question for the answer, or `None` for a question graded by hand.
fn score(question: &Question, answer: Option<&Value>) -> Option<Decimal> {
    let full = question.points;
    let none = Decimal::ZERO;
    let Some(answer) = answer else {
        return (question.kind != QuestionType::LongText).then_some(none);
    };
    match question.kind {
        QuestionType::LongText => None,
        QuestionType::SingleChoice => {
            let picked = choice_list(answer).unwrap_or_default();
            Some(
                if picked.len() == 1 && question.correct.iter().any(|c| c == picked[0]) {
                    full
                } else {
                    none
                },
            )
        }
        QuestionType::MultipleChoice => {
            let picked = choice_list(answer).unwrap_or_default();
            let good = picked
                .iter()
                .filter(|id| question.correct.iter().any(|c| c == **id))
                .count();
            let bad = picked.len() - good;
            Some(if question.partial_credit {
                let net = i64::try_from(good).unwrap_or(0) - i64::try_from(bad).unwrap_or(0);
                let right = i64::try_from(question.correct.len()).unwrap_or(i64::MAX);
                (full * Decimal::from(net.max(0)) / Decimal::from(right)).round_dp(2)
            } else if good == question.correct.len() && bad == 0 {
                full
            } else {
                none
            })
        }
        QuestionType::Number => {
            let given = answer.as_str().and_then(parse_number);
            Some(match (given, question.answer) {
                (Some(given), Some(expected)) if (given - expected).abs() <= question.tolerance => {
                    full
                }
                _ => none,
            })
        }
        QuestionType::ShortText => {
            let given = answer.as_str().map(|text| text.trim().to_lowercase());
            Some(
                if given.is_some_and(|text| question.accepted.contains(&text)) {
                    full
                } else {
                    none
                },
            )
        }
    }
}

/// Scores the answers of an attempt (ADR-0050 point 4). `None` when the config cannot be read.
pub fn auto_grade(config: &Map<String, Value>, content: &Map<String, Value>) -> Option<AutoGrade> {
    let quiz = parse(config, None).ok()?;
    let answers = content.get("answers").and_then(Value::as_object);
    let mut scores = BTreeMap::new();
    let mut complete = true;
    for question in &quiz.questions {
        match score(question, answers.and_then(|a| a.get(&question.id))) {
            Some(score) => {
                scores.insert(question.id.clone(), score);
            }
            None => complete = false,
        }
    }
    let total = scores.values().copied().sum();
    Some(AutoGrade {
        scores,
        total,
        complete,
    })
}

/// One question of a result as the student sees it (ADR-0050 point 8).
#[derive(Debug, Clone, PartialEq)]
pub struct ResultQuestion {
    pub id: String,
    pub points: Decimal,
    pub got: Option<Decimal>,
    pub answer: Option<Value>,
    /// The answer key, only when the quiz shows it.
    pub correct: Option<Value>,
    pub explanation: Option<Value>,
}

/// The questions of a result: the student's answers, the stored scores and, when the quiz shows
/// them, the correct answers and explanations.
pub fn result_questions(
    config: &Map<String, Value>,
    content: &Map<String, Value>,
    auto_scores: &Map<String, Value>,
) -> Option<Vec<ResultQuestion>> {
    let quiz = parse(config, None).ok()?;
    let answers = content.get("answers").and_then(Value::as_object);
    let raw_questions = config.get("questions").and_then(Value::as_array)?;
    let questions = quiz
        .questions
        .iter()
        .zip(raw_questions)
        .map(|(question, raw)| {
            let correct = quiz.show_correct.then(|| match question.kind {
                QuestionType::SingleChoice | QuestionType::MultipleChoice => {
                    Some(json!(question.correct))
                }
                QuestionType::Number => question
                    .answer
                    .map(|answer| Value::String(answer.normalize().to_string())),
                QuestionType::ShortText => raw.get("accepted").cloned(),
                QuestionType::LongText => None,
            });
            ResultQuestion {
                id: question.id.clone(),
                points: question.points,
                got: auto_scores
                    .get(&question.id)
                    .and_then(Value::as_str)
                    .and_then(|text| text.parse().ok()),
                answer: answers.and_then(|a| a.get(&question.id)).cloned(),
                correct: correct.flatten(),
                explanation: quiz
                    .show_correct
                    .then(|| raw.get("explanation").cloned())
                    .flatten()
                    .filter(|value| !value.is_null()),
            }
        })
        .collect();
    Some(questions)
}

#[cfg(test)]
mod tests;
