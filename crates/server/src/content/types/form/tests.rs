use serde_json::{Map, Value, json};

use super::*;

fn obj(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap()
}

fn field(id: &str, kind: &str) -> Value {
    json!({"id": id, "type": kind, "label": format!("Label {id}"), "required": false})
}

fn choice(id: &str, rules: Value) -> Value {
    let mut f = field(id, "choice");
    f["options"] =
        json!([{"id": "x", "text": "X"}, {"id": "y", "text": "Y"}, {"id": "z", "text": "Z"}]);
    f["rules"] = rules;
    f
}

fn form(fields: Vec<Value>) -> Value {
    json!({"v": 1, "note": "Fill it in", "fields": fields})
}

fn message(config: &Value) -> String {
    validate_config(obj(config)).unwrap_err().to_string()
}

fn check(config: &Value, answers: Value, stage: Stage) -> Result<ValidContent, DocumentErrors> {
    validate_submission(obj(config), &json!({"answers": answers}), stage)
}

fn paths(config: &Value, answers: Value, stage: Stage) -> Vec<String> {
    check(config, answers, stage)
        .unwrap_err()
        .iter()
        .map(|error| error.path.clone())
        .collect()
}

fn every_kind() -> Value {
    let mut short = field("name", "short");
    short["required"] = json!(true);
    short["rules"] = json!({"max_length": 10});
    let mut people = field("team", "people");
    people["rules"] = json!({"min": 1, "max": 2});
    let mut num = field("size", "number");
    num["rules"] = json!({"min": "1", "max": "10.5"});
    form(vec![
        short,
        field("about", "long"),
        choice("topic", json!({"exclusive": true})),
        choice("tags", json!({"multiple": true})),
        people,
        field("repo", "link"),
        num,
        field("when", "date"),
    ])
}

#[test]
fn a_form_of_every_field_type_is_valid() {
    assert_eq!(validate_config(obj(&every_kind())), Ok(()));
}

#[test]
fn a_config_with_the_wrong_shape_is_refused() {
    let mut config = form(vec![field("a", "short")]);
    config["surprise"] = json!(true);
    assert!(message(&config).contains("wrong shape"));
    let mut config = form(vec![field("a", "short")]);
    config["v"] = json!(3);
    assert!(message(&config).contains("`v` must be 1"));
    assert!(message(&form(vec![])).contains("1 to 50"));
    let mut config = form(vec![field("a", "short")]);
    config["note"] = json!("x".repeat(1001));
    assert!(message(&config).contains("note"));
}

#[test]
fn fields_are_checked() {
    for bad in ["", "a b"] {
        assert!(
            message(&form(vec![field(bad, "short")])).contains("fields[0].id"),
            "{bad:?}"
        );
    }
    assert!(message(&form(vec![field("a", "short"), field("a", "long")])).contains("fields[1].id"));
    let mut f = field("a", "short");
    f["label"] = json!("  ");
    assert!(message(&form(vec![f])).contains("label"));
    let mut f = field("a", "short");
    f["help"] = json!("x".repeat(501));
    assert!(message(&form(vec![f])).contains("help"));
    let mut f = field("a", "short");
    f["options"] = json!([{"id": "x", "text": "X"}]);
    assert!(message(&form(vec![f])).contains("options"));
}

#[test]
fn rules_belong_to_their_type_and_are_in_range() {
    let cases = [
        ("short", json!({"exclusive": true}), "exclusive"),
        ("short", json!({"max_length": 0}), "max_length"),
        ("short", json!({"max_length": 501}), "max_length"),
        ("long", json!({"max_length": 10_001}), "max_length"),
        ("people", json!({"min": 3, "max": 2}), "min"),
        ("people", json!({"max": 11}), "max"),
        ("number", json!({"min": 5}), "min"),
        ("number", json!({"min": "9", "max": "1"}), "min"),
        ("link", json!({"max_length": 5}), "max_length"),
        ("date", json!({"min": "1"}), "min"),
    ];
    for (kind, rules, name) in cases {
        let mut f = field("a", kind);
        f["rules"] = rules.clone();
        assert!(message(&form(vec![f])).contains(name), "{kind} {rules}");
    }
    let mut f = choice("a", json!({"exclusive": "yes"}));
    f["rules"] = json!({"exclusive": "yes"});
    assert!(message(&form(vec![f])).contains("exclusive"));
}

#[test]
fn a_choice_needs_two_to_twenty_distinct_options() {
    let mut f = choice("a", json!({}));
    f["options"] = json!([{"id": "x", "text": "X"}]);
    assert!(message(&form(vec![f])).contains("2 to 20"));
    let mut f = choice("a", json!({}));
    f["options"] = json!([{"id": "x", "text": "X"}, {"id": "x", "text": "Y"}]);
    assert!(message(&form(vec![f])).contains("options[1]"));
    let mut f = choice("a", json!({}));
    f["options"] = json!([{"id": "x", "text": "X"}, {"id": "y", "text": " "}]);
    assert!(message(&form(vec![f])).contains("options[1]"));
    let mut f = field("a", "choice");
    f.as_object_mut().unwrap().remove("options");
    assert!(message(&form(vec![f])).contains("`fields[0].options` is required"));
}

#[test]
fn a_draft_may_be_unfinished_and_a_submit_may_not() {
    let config = every_kind();
    let draft = check(&config, json!({"about": "text"}), Stage::Draft).unwrap();
    assert!(!draft.is_blank());
    assert!(check(&config, json!({}), Stage::Draft).unwrap().is_blank());
    assert_eq!(
        paths(&config, json!({"about": "text"}), Stage::Submit),
        vec!["answers.name"]
    );
    let submit = check(
        &config,
        json!({"name": "Team A", "team": [], "tags": ["x", "z"]}),
        Stage::Submit,
    );
    assert!(submit.is_ok(), "{submit:?}");
}

#[test]
fn a_people_field_enforces_its_minimum_only_when_submitted() {
    let mut people = field("team", "people");
    people["rules"] = json!({"min": 2, "max": 3});
    let config = form(vec![people]);
    let one = json!(["00000000-0000-7000-8000-000000000001"]);
    assert!(check(&config, json!({"team": one.clone()}), Stage::Draft).is_ok());
    assert_eq!(
        paths(&config, json!({"team": one}), Stage::Submit),
        vec!["answers.team"]
    );
    let four = json!([
        "00000000-0000-7000-8000-000000000001",
        "00000000-0000-7000-8000-000000000002",
        "00000000-0000-7000-8000-000000000003",
        "00000000-0000-7000-8000-000000000004"
    ]);
    assert_eq!(
        paths(&config, json!({"team": four}), Stage::Draft),
        vec!["answers.team"]
    );
    let twice = json!([
        "00000000-0000-7000-8000-000000000001",
        "00000000-0000-7000-8000-000000000001"
    ]);
    assert_eq!(
        paths(&config, json!({"team": twice}), Stage::Draft),
        vec!["answers.team"]
    );
    assert_eq!(
        paths(&config, json!({"team": ["not-an-id"]}), Stage::Draft),
        vec!["answers.team"]
    );
}

#[test]
fn answers_are_checked_against_their_field() {
    let config = every_kind();
    let bad = [
        (json!({"nope": "x"}), "answers.nope"),
        (json!({"name": "far too long for it"}), "answers.name"),
        (json!({"name": 5}), "answers.name"),
        (json!({"topic": ["q"]}), "answers.topic"),
        (json!({"topic": ["x", "y"]}), "answers.topic"),
        (json!({"topic": "x"}), "answers.topic"),
        (json!({"tags": ["x", "x"]}), "answers.tags"),
        (json!({"repo": "ftp://x"}), "answers.repo"),
        (json!({"repo": "not a url"}), "answers.repo"),
        (json!({"size": "0"}), "answers.size"),
        (json!({"size": "11"}), "answers.size"),
        (json!({"size": "ten"}), "answers.size"),
        (json!({"when": "2026-13-01"}), "answers.when"),
        (json!({"when": "23.10.2026"}), "answers.when"),
        (json!({"about": "bad\u{0}char"}), "answers.about"),
    ];
    for (given, path) in bad {
        assert_eq!(
            paths(&config, given.clone(), Stage::Draft),
            vec![path],
            "{given}"
        );
    }
    let good = check(
        &config,
        json!({
            "name": " Team A ", "about": "line one\nline two", "topic": ["y"], "tags": ["x", "y"],
            "repo": "https://example.org/r", "size": "10,5", "when": "2026-10-23", "team": []
        }),
        Stage::Draft,
    )
    .unwrap();
    let stored = good.as_body()["answers"].as_object().unwrap();
    assert_eq!(stored["name"], json!("Team A"));
    assert_eq!(stored["about"], json!("line one\nline two"));
    assert!(!stored.contains_key("team"), "an empty list is not stored");
}

#[test]
fn content_that_is_not_an_answers_object_is_refused() {
    let config = form(vec![field("a", "short")]);
    for content in [json!([]), json!({"answers": []}), json!({"a": "x"})] {
        assert!(
            validate_submission(obj(&config), &content, Stage::Draft).is_err(),
            "{content}"
        );
    }
    assert!(
        validate_submission(obj(&config), &json!({}), Stage::Draft)
            .unwrap()
            .is_blank()
    );
}

#[test]
fn claims_list_the_people_and_the_exclusive_options_chosen() {
    let config = every_kind();
    let id = "00000000-0000-7000-8000-000000000001";
    let valid = check(
        &config,
        json!({"name": "A", "topic": ["y"], "tags": ["x"], "team": [id]}),
        Stage::Submit,
    )
    .unwrap();
    let claims = claims(obj(&config), valid.as_body());
    assert_eq!(claims.people, vec![id.parse::<Uuid>().unwrap()]);
    assert_eq!(claims.exclusive, vec![("topic".to_owned(), "y".to_owned())]);
    let none = check(&config, json!({"name": "A"}), Stage::Submit).unwrap();
    assert!(super::claims(obj(&config), none.as_body()).is_empty());
}

#[test]
fn the_structure_ignores_wording_but_not_fields_rules_or_options() {
    let config = every_kind();
    let mut reworded = config.clone();
    reworded["note"] = json!("A new note");
    reworded["fields"][0]["label"] = json!("Another label");
    reworded["fields"][0]["help"] = json!("Some help");
    reworded["fields"][2]["options"][0]["text"] = json!("Renamed");
    assert_eq!(structure(obj(&config)), structure(obj(&reworded)));

    let edits: [fn(&mut Value); 5] = [
        |c| c["fields"][0]["required"] = json!(false),
        |c| c["fields"][0]["rules"] = json!({"max_length": 11}),
        |c| c["fields"][2]["options"] = json!([{"id": "x", "text": "X"}, {"id": "y", "text": "Y"}]),
        |c| c["fields"][1]["id"] = json!("renamed"),
        |c| {
            let first = c["fields"][0].clone();
            c["fields"][0] = c["fields"][1].clone();
            c["fields"][1] = first;
        },
    ];
    for (n, edit) in edits.into_iter().enumerate() {
        let mut changed = config.clone();
        edit(&mut changed);
        assert_ne!(
            structure(obj(&config)),
            structure(obj(&changed)),
            "edit {n}"
        );
    }
}

#[test]
fn computed_facts_are_added_for_the_reader_and_stripped_on_the_way_back() {
    let me = Uuid::now_v7();
    let other = Uuid::now_v7();
    let config = every_kind();
    let answered: HashSet<String> = ["name".to_owned()].into();
    let mut holders = Holders::new();
    holders.insert(
        ("topic".to_owned(), "x".to_owned()),
        (other, "Ann Lee".to_owned()),
    );
    holders.insert(
        ("topic".to_owned(), "y".to_owned()),
        (me, "Me Myself".to_owned()),
    );

    let mut student = obj(&config).clone();
    decorate(&mut student, &answered, &holders, false, me);
    let topic = &student["fields"][2]["options"];
    assert_eq!(topic[0]["taken"], json!(true));
    assert_eq!(
        topic[1]["taken"],
        json!(false),
        "my own option is not taken for me"
    );
    assert_eq!(topic[2]["taken"], json!(false));
    let seen = Value::Object(student.clone()).to_string();
    assert!(
        !seen.contains("taken_by") && !seen.contains("Ann") && !seen.contains("has_answers"),
        "{seen}"
    );
    assert!(
        student["fields"][3]["options"][0].get("taken").is_none(),
        "only exclusive fields"
    );

    let mut staff = obj(&config).clone();
    decorate(&mut staff, &answered, &holders, true, Uuid::now_v7());
    assert_eq!(
        staff["fields"][2]["options"][0]["taken_by"],
        json!("Ann Lee")
    );
    assert_eq!(staff["fields"][2]["options"][2]["taken_by"], Value::Null);
    assert_eq!(staff["fields"][0]["has_answers"], json!(true));
    assert_eq!(staff["fields"][1]["has_answers"], json!(false));

    strip_computed(&mut staff);
    assert_eq!(Value::Object(staff), config);
}
