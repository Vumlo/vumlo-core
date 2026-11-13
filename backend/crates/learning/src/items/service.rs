use sqlx::PgConnection;
use uuid::Uuid;

use crate::LearningError;
use crate::assessment::{self, PublishedScore, QuizResult};
use crate::content::{
    self, AssignmentFacts, BuiltinType, Item, ItemPatch, ItemRules, NewItemInput,
};
use crate::files::{self, UsageOwner};
use crate::gradebook;
use access::Actor;

/// Deletes the module with its sub-modules and items. Drafts of students and empty grade rows go
/// with it; it is refused when an assignment below has a grade or a submitted attempt.
///
/// `content` authorizes and locks every assignment below first, then the grades and attempts are
/// asked in later statements, so a grade write or a submit in flight is seen (ADR-0028,
/// ADR-0035).
pub async fn delete_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), LearningError> {
    let deletion = content::begin_delete_module(conn, actor, id).await?;
    if gradebook::has_grades_in_module(conn, id).await? {
        return Err(LearningError::HasGrades);
    }
    if assessment::has_submitted_in_module(conn, id).await? {
        return Err(LearningError::HasSubmissions);
    }
    // The empty rows that are left go with their history; the grades table has no foreign key to
    // cascade through. Drafts without an attempt go with the module.
    gradebook::purge_module(conn, id).await?;
    assessment::purge_module(conn, id).await?;
    content::finish_delete_module(conn, actor, deletion).await?;
    Ok(())
}

/// Deletes a `text` item or an assignment. Drafts of students and empty grade rows go with an
/// assignment; it is refused when the assignment has a grade or a submitted attempt.
pub async fn delete_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), LearningError> {
    let deletion = content::begin_delete_item(conn, actor, id).await?;
    if deletion.is_assignment() {
        if gradebook::has_grades_for_assignment(conn, id).await? {
            return Err(LearningError::HasGrades);
        }
        if assessment::has_submitted_for_assignment(conn, id).await? {
            return Err(LearningError::HasSubmissions);
        }
        gradebook::purge_assignment(conn, id).await?;
        assessment::purge_assignment(conn, id).await?;
    }
    content::finish_delete_item(conn, actor, deletion).await?;
    Ok(())
}

/// Creates a draft item as the last of a module. `content` authorizes and validates, the files it
/// names are checked and locked (`files::check`), and `content` inserts the item and attaches them
/// in the same transaction (ADR-0069): the lock is held until the handler commits, so no one else
/// can attach an upload that this item took.
pub async fn create_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    module_id: Uuid,
    input: NewItemInput,
) -> Result<Item, LearningError> {
    let creation = content::begin_create_item(conn, actor, rules, module_id, input).await?;
    let owner = UsageOwner::Item(creation.id());
    let verdict = files::check(conn, actor.user_id(), &owner, creation.files()).await?;
    Ok(content::finish_create_item(conn, actor, creation, &verdict).await?)
}

/// Changes an item. `content` validates and takes the assignment row lock; the files the new body
/// and config name are checked and locked after it (the order of the locks is the owner's row,
/// then the files); what only the gradebook and the attempts know (work submitted, the highest
/// score given) is read after the lock, and only when the update asks for it, then `content`
/// decides and writes (ADR-0023, ADR-0035, ADR-0069).
pub async fn update_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    id: Uuid,
    patch: ItemPatch,
) -> Result<Item, LearningError> {
    let update = content::begin_update_item(conn, actor, rules, id, patch).await?;
    let verdict =
        files::check(conn, actor.user_id(), &UsageOwner::Item(id), update.files()).await?;
    let has_submissions =
        update.asks_submissions() && assessment::has_submitted_for_assignment(conn, id).await?;
    let highest_score = if update.asks_scores() {
        gradebook::max_score(conn, id).await?
    } else {
        None
    };
    let facts = AssignmentFacts {
        has_submissions,
        highest_score,
    };
    Ok(content::finish_update_item(conn, actor, update, facts, &verdict).await?)
}

/// An item as the actor may read it. A form that is open to the reader also shows which of its
/// fields have answers and who holds each exclusive option (ADR-0051 point 5).
pub async fn get(conn: &mut PgConnection, actor: &Actor, id: Uuid) -> Result<Item, LearningError> {
    let read = content::get_item(conn, actor, id).await?;
    let mut item = read.item;
    if let (Some(view), Some(assignment)) = (read.form, &item.assignment) {
        let facts = assessment::form_facts(conn, id, BuiltinType::Form, &assignment.config).await?;
        content::decorate_form(&mut item, view, &facts.answered, &facts.holders);
    }
    Ok(item)
}

/// The caller's result of a quiz: their latest attempt, the server's score of each question and the
/// published grade. Only once a score is published (ADR-0050 point 8). `assessment` authorizes and
/// locks the assignment, the gradebook says what was published, `assessment` builds the result.
pub async fn result(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<QuizResult, LearningError> {
    let start = assessment::begin_quiz_result(conn, actor, assignment_id).await?;
    let published = gradebook::published_grade(conn, assignment_id, actor.user_id())
        .await?
        .map(|grade| PublishedScore {
            score: grade.score,
            feedback: grade.feedback,
            published_at: grade.published_at,
        });
    Ok(assessment::finish_quiz_result(conn, actor, start, published).await?)
}
