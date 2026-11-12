//! The owners (items, submissions) are real rows made through the services; the references between
//! them and the files are written through the repo functions of `content` and `assessment`.
use std::sync::{Arc, LazyLock};

use chrono::{Duration, Utc};
use serde_json::{Map, json};
use sqlx::PgPool;
use test_support::db_test;

use super::*;
use crate::content::{
    Assignment, BuiltinType, ContentTitle, EffectiveProfiles, ItemPatch, ItemRules, MaxPoints,
};
use crate::files::sweep;
use crate::teaching::{self, CourseTitle};
use access::CourseRole;
use auth::{self, Email, NewUser, PersonName};
use common::Role;
use files::{Storage, Stored};
use object_store::memory::InMemory;
use vumlo_config::GradingScale;

static PROFILES: LazyLock<EffectiveProfiles> = LazyLock::new(EffectiveProfiles::unrestricted);

fn rules() -> ItemRules<'static> {
    ItemRules {
        scale: GradingScale::Hundred,
        profiles: &PROFILES,
    }
}

struct Fixture {
    pool: PgPool,
    owner: Actor,
    student: Actor,
    classmate: Actor,
    /// A teacher who is not on the course.
    stranger: Actor,
    course: Uuid,
    /// A published item in a published module.
    lesson: Uuid,
    /// A published text assignment in the same module.
    assignment: Uuid,
}

async fn fixture(pool: PgPool) -> Fixture {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let mut create = async |email: &str, role: Role| {
        auth::create_user_unaudited_for_tests(
            &mut tx,
            NewUser {
                email: Email::parse(email).unwrap(),
                name: PersonName::name("Test").unwrap(),
                surname: PersonName::surname("User").unwrap(),
                role,
            },
            hash.clone(),
        )
        .await
        .unwrap()
    };
    let owner_user = create("owner@x.edu", Role::Teacher).await;
    let student_user = create("student@x.edu", Role::Student).await;
    let classmate_user = create("classmate@x.edu", Role::Student).await;
    let stranger_user = create("stranger@x.edu", Role::Teacher).await;
    let actor = |id, role| Actor::for_tests(id, role, Uuid::now_v7());
    let owner = actor(owner_user.id, Role::Teacher);
    let course = teaching::create_course(
        &mut tx,
        &owner,
        CourseTitle::parse("Algebra").unwrap(),
        None,
    )
    .await
    .unwrap();
    for user in [&student_user, &classmate_user] {
        teaching::put_member(&mut tx, &owner, course.id, user.id, CourseRole::Student)
            .await
            .unwrap();
    }
    teaching::publish_course(&mut tx, &owner, course.id)
        .await
        .unwrap();
    let module = content::create_module(
        &mut tx,
        &owner,
        course.id,
        ContentTitle::parse("M").unwrap(),
        None,
    )
    .await
    .unwrap();
    content::publish_module(&mut tx, &owner, module.id, true)
        .await
        .unwrap();
    let lesson = published_item(&mut tx, &owner, module.id, None).await;
    let assignment = content::create_item(
        &mut tx,
        &owner,
        rules(),
        module.id,
        ContentTitle::parse("Essay").unwrap(),
        None,
        Some(Assignment {
            assignment_type: BuiltinType::Text,
            config: Map::new(),
            max_points: MaxPoints::parse("10").unwrap(),
            is_bonus: false,
            settings: Default::default(),
            class_on: None,
        }),
    )
    .await
    .unwrap();
    content::publish_item(&mut tx, &owner, assignment.id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        pool,
        student: actor(student_user.id, Role::Student),
        classmate: actor(classmate_user.id, Role::Student),
        stranger: actor(stranger_user.id, Role::Teacher),
        owner,
        course: course.id,
        lesson: lesson.id,
        assignment: assignment.id,
    }
}

/// A published text item in `module`, scheduled for `publish_at` when given.
async fn published_item(
    conn: &mut PgConnection,
    owner: &Actor,
    module: Uuid,
    publish_at: Option<chrono::DateTime<Utc>>,
) -> content::Item {
    let item = content::create_item(
        conn,
        owner,
        rules(),
        module,
        ContentTitle::parse("Lesson").unwrap(),
        None,
        None,
    )
    .await
    .unwrap();
    if publish_at.is_some() {
        let patch = ItemPatch {
            publish_at: Some(publish_at),
            ..ItemPatch::default()
        };
        content::update_item(conn, owner, rules(), item.id, patch)
            .await
            .unwrap();
    }
    content::publish_item(conn, owner, item.id).await.unwrap()
}

/// A file row uploaded by `uploader`, referenced by nobody.
async fn file(pool: &PgPool, uploader: &Actor) -> Uuid {
    let id = Uuid::now_v7();
    let stored = Stored {
        id,
        size: 3,
        sha256: "00".to_owned(),
        head: b"abc".to_vec(),
    };
    let mut conn = pool.acquire().await.unwrap();
    files::insert(
        &mut conn,
        &stored,
        uploader.user_id(),
        "a.txt",
        "text/plain",
    )
    .await
    .unwrap();
    id
}

async fn item_names(pool: &PgPool, item: Uuid, published: bool, file: Uuid) {
    let mut conn = pool.acquire().await.unwrap();
    content::replace_item_files(&mut conn, item, published, &[file])
        .await
        .unwrap();
}

/// The id of the student's draft or latest attempt on the assignment.
async fn submission_of(pool: &PgPool, assignment: Uuid, student: &Actor) -> Uuid {
    sqlx::query_scalar!(
        "SELECT id FROM assessment.submissions WHERE assignment_id = $1 AND student_id = $2",
        assignment,
        student.user_id()
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The student saves a draft whose one block is a `file` block naming `file`.
async fn draft_naming(f: &Fixture, file: Uuid) -> Uuid {
    let mut conn = f.pool.acquire().await.unwrap();
    let document = json!({"v": 1, "attachments": [], "blocks": [
        {"id": "i", "type": "file", "props": {"file_id": file}}]});
    assessment::save_draft(
        &mut conn,
        &f.student,
        f.assignment,
        &document,
        &PROFILES,
        Utc::now(),
    )
    .await
    .unwrap();
    submission_of(&f.pool, f.assignment, &f.student).await
}

async fn submit(f: &Fixture) {
    let mut conn = f.pool.acquire().await.unwrap();
    assessment::submit(&mut conn, &f.student, f.assignment, &PROFILES, Utc::now())
        .await
        .unwrap();
}

async fn read(pool: &PgPool, actor: &Actor, file: Uuid) -> Result<files::File, FileError> {
    let mut conn = pool.acquire().await.unwrap();
    open_for_read(&mut conn, actor, file).await
}

fn not_found(result: Result<files::File, FileError>) -> bool {
    matches!(result, Err(FileError::NotFound))
}

#[test]
fn an_owner_is_parsed_from_the_for_parameter() {
    let id = Uuid::now_v7();
    assert_eq!(Owner::parse(&format!("item:{id}")), Some(Owner::Item(id)));
    assert_eq!(
        Owner::parse(&format!("assignment:{id}")),
        Some(Owner::Assignment(id))
    );
    for bad in [
        "",
        "item",
        "item:",
        "item:x",
        &format!("course:{id}"),
        &id.to_string(),
    ] {
        assert_eq!(Owner::parse(bad), None, "{bad}");
    }
}

#[tokio::test]
async fn uploader_reads_an_unreferenced_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        let got = read(&f.pool, &f.student, id).await.unwrap();
        assert_eq!(got.id, id);
    })
    .await;
}

#[tokio::test]
async fn other_user_gets_not_found_for_an_unreferenced_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        for other in [&f.classmate, &f.owner, &f.stranger] {
            assert!(not_found(read(&f.pool, other, id).await));
        }
        assert!(not_found(read(&f.pool, &f.student, Uuid::now_v7()).await));
    })
    .await;
}

#[tokio::test]
async fn student_gets_not_found_for_working_copy_file_of_a_published_item() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, id).await;
        assert!(not_found(read(&f.pool, &f.student, id).await));
    })
    .await;
}

#[tokio::test]
async fn student_reads_published_row_of_a_visible_item() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, true, id).await;
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
        assert_eq!(read(&f.pool, &f.classmate, id).await.unwrap().id, id);
        // Not a member of the course: the course does not exist for them.
        assert!(not_found(read(&f.pool, &f.stranger, id).await));
    })
    .await;
}

#[tokio::test]
async fn student_gets_not_found_when_item_not_yet_published_or_before_publish_at() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let mut tx = f.pool.begin().await.unwrap();
        let module = content::create_module(
            &mut tx,
            &f.owner,
            f.course,
            ContentTitle::parse("Hidden").unwrap(),
            None,
        )
        .await
        .unwrap();
        let draft = content::create_item(
            &mut tx,
            &f.owner,
            rules(),
            module.id,
            ContentTitle::parse("Draft").unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        let scheduled = published_item(
            &mut tx,
            &f.owner,
            module.id,
            Some(Utc::now() + Duration::days(1)),
        )
        .await;
        let hidden_module = published_item(&mut tx, &f.owner, module.id, None).await;
        tx.commit().await.unwrap();
        // The item is a draft; the item is scheduled in a published module; the item is published
        // in a module that is still a draft.
        let visible_module = content::create_module(
            &mut f.pool.acquire().await.unwrap(),
            &f.owner,
            f.course,
            ContentTitle::parse("Open").unwrap(),
            None,
        )
        .await
        .unwrap();
        content::publish_module(
            &mut f.pool.acquire().await.unwrap(),
            &f.owner,
            visible_module.id,
            true,
        )
        .await
        .unwrap();
        let scheduled_in_open = published_item(
            &mut f.pool.acquire().await.unwrap(),
            &f.owner,
            visible_module.id,
            Some(Utc::now() + Duration::days(1)),
        )
        .await;
        let draft_in_open = content::create_item(
            &mut f.pool.acquire().await.unwrap(),
            &f.owner,
            rules(),
            visible_module.id,
            ContentTitle::parse("Draft").unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        let hidden = [
            draft.id,
            scheduled.id,
            hidden_module.id,
            scheduled_in_open.id,
            draft_in_open.id,
        ];
        for item in hidden {
            let id = file(&f.pool, &f.owner).await;
            item_names(&f.pool, item, true, id).await;
            assert!(not_found(read(&f.pool, &f.student, id).await), "{item}");
            // Staff see it all.
            assert_eq!(read(&f.pool, &f.owner, id).await.unwrap().id, id);
        }
    })
    .await;
}

#[tokio::test]
async fn staff_reads_working_copy_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, id).await;
        assert_eq!(read(&f.pool, &f.owner, id).await.unwrap().id, id);
        assert!(not_found(read(&f.pool, &f.stranger, id).await));
    })
    .await;
}

#[tokio::test]
async fn author_reads_own_submission_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        draft_naming(&f, id).await;
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
        submit(&f).await;
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
        // Staff read it once it is an attempt.
        assert_eq!(read(&f.pool, &f.owner, id).await.unwrap().id, id);
    })
    .await;
}

#[tokio::test]
async fn other_student_gets_not_found_for_a_submission_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        draft_naming(&f, id).await;
        submit(&f).await;
        assert!(not_found(read(&f.pool, &f.classmate, id).await));
        assert!(not_found(read(&f.pool, &f.stranger, id).await));
    })
    .await;
}

#[tokio::test]
async fn staff_gets_not_found_for_a_draft_submission_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        draft_naming(&f, id).await;
        assert!(not_found(read(&f.pool, &f.owner, id).await));
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
    })
    .await;
}

#[tokio::test]
async fn any_referencing_row_that_grants_access_is_enough() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        draft_naming(&f, id).await;
        // Also named by the working copy of a lesson, which a student may not read.
        item_names(&f.pool, f.lesson, false, id).await;
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
        assert_eq!(read(&f.pool, &f.owner, id).await.unwrap().id, id);
        assert!(not_found(read(&f.pool, &f.classmate, id).await));
    })
    .await;
}

async fn usable(
    f: &Fixture,
    actor: &Actor,
    owner: UsageOwner,
    ids: &[Uuid],
) -> Result<(), FileError> {
    let mut conn = f.pool.acquire().await.unwrap();
    ensure_usable(&mut conn, actor.user_id(), &owner, ids).await
}

#[tokio::test]
async fn ensure_usable_accepts_own_unreferenced_upload() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        usable(&f, &f.owner, UsageOwner::Item(f.lesson), &[id])
            .await
            .unwrap();
        usable(&f, &f.owner, UsageOwner::Item(f.lesson), &[])
            .await
            .unwrap();
        // Someone else's unreferenced upload is not theirs to name.
        let error = usable(&f, &f.stranger, UsageOwner::Item(f.lesson), &[id])
            .await
            .unwrap_err();
        assert!(matches!(error, FileError::Unusable { id: bad } if bad == id));
    })
    .await;
}

#[tokio::test]
async fn ensure_usable_accepts_file_already_on_this_owner() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, id).await;
        // Another editor of the same item may keep naming it.
        usable(&f, &f.stranger, UsageOwner::Item(f.lesson), &[id])
            .await
            .unwrap();
        let theirs = file(&f.pool, &f.student).await;
        let submission = draft_naming(&f, theirs).await;
        usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: submission,
                assignment_id: f.assignment,
            },
            &[theirs],
        )
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_file_also_held_by_another_owner_is_still_usable_by_an_owner_that_holds_it() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, id).await;
        item_names(&f.pool, f.assignment, false, id).await;
        // Each of the two holders may keep naming the file, though the other one holds it too.
        usable(&f, &f.owner, UsageOwner::Item(f.lesson), &[id])
            .await
            .unwrap();
        usable(&f, &f.owner, UsageOwner::Item(f.assignment), &[id])
            .await
            .unwrap();
        // An owner that does not hold it still cannot borrow it, even as the uploader.
        let stranger_submission = UsageOwner::Submission {
            id: Uuid::now_v7(),
            assignment_id: f.assignment,
        };
        let error = usable(&f, &f.owner, stranger_submission, &[id])
            .await
            .unwrap_err();
        assert!(matches!(error, FileError::Unusable { id: bad } if bad == id));
    })
    .await;
}

#[tokio::test]
async fn ensure_usable_rejects_file_of_another_owner_and_unknown_id() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let mine = file(&f.pool, &f.student).await;
        let submission = draft_naming(&f, mine).await;
        // The student cannot hang the same file on a lesson, and a teacher cannot hang it on
        // another item or on the classmate's submission, though they would pass as uploader.
        let on_lesson = usable(&f, &f.student, UsageOwner::Item(f.lesson), &[mine]).await;
        assert!(matches!(on_lesson, Err(FileError::Unusable { id }) if id == mine));
        let other = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, other).await;
        let on_assignment = usable(&f, &f.owner, UsageOwner::Item(f.assignment), &[other]).await;
        assert!(matches!(on_assignment, Err(FileError::Unusable { id }) if id == other));
        let on_submission = usable(
            &f,
            &f.owner,
            UsageOwner::Submission {
                id: submission,
                assignment_id: f.assignment,
            },
            &[other],
        )
        .await;
        assert!(matches!(on_submission, Err(FileError::Unusable { id }) if id == other));
        // Unknown id; and the first bad id of a list is the one reported.
        let unknown = Uuid::now_v7();
        let error = usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: submission,
                assignment_id: f.assignment,
            },
            &[mine, unknown],
        )
        .await
        .unwrap_err();
        assert!(matches!(error, FileError::Unusable { id } if id == unknown));
    })
    .await;
}

/// `actor` saves a draft on `assignment` whose one block is a `file` block naming `file`.
async fn save_draft_naming(
    f: &Fixture,
    actor: &Actor,
    assignment: Uuid,
    file: Uuid,
) -> Result<(), AssessmentError> {
    let mut conn = f.pool.acquire().await.unwrap();
    let document = json!({"v": 1, "attachments": [], "blocks": [
        {"id": "i", "type": "file", "props": {"file_id": file}}]});
    assessment::save_draft(
        &mut conn,
        actor,
        assignment,
        &document,
        &PROFILES,
        Utc::now(),
    )
    .await
    .map(|_| ())
}

/// The ids of the submission rows of `actor` on `assignment`, oldest first.
async fn rows_of(pool: &PgPool, assignment: Uuid, actor: &Actor) -> Vec<Uuid> {
    sqlx::query_scalar!(
        "SELECT id FROM assessment.submissions WHERE assignment_id = $1 AND student_id = $2
         ORDER BY id",
        assignment,
        actor.user_id()
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn holders(pool: &PgPool, file: Uuid) -> Vec<Uuid> {
    let mut conn = pool.acquire().await.unwrap();
    let mut ids = repo::submission_references(&mut conn, file).await.unwrap();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_new_draft_may_name_a_file_of_the_authors_earlier_attempt() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, f.assignment, id)
            .await
            .unwrap();
        submit(&f).await;
        // The resubmission starts from the earlier answer: a new draft row names the same file.
        save_draft_naming(&f, &f.student, f.assignment, id)
            .await
            .unwrap();
        let rows = rows_of(&f.pool, f.assignment, &f.student).await;
        assert_eq!(rows.len(), 2, "the attempt and the new draft");
        assert_eq!(
            holders(&f.pool, id).await,
            rows,
            "each keeps its own reference"
        );
        // The student still reads it, and staff read it through the attempt.
        assert_eq!(read(&f.pool, &f.student, id).await.unwrap().id, id);
        assert_eq!(read(&f.pool, &f.owner, id).await.unwrap().id, id);
    })
    .await;
}

#[tokio::test]
async fn a_draft_cannot_name_a_file_of_another_students_attempt_or_of_another_assignment() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        // The classmate's attempt holds a file; the student has their own draft row.
        let theirs = file(&f.pool, &f.classmate).await;
        save_draft_naming(&f, &f.classmate, f.assignment, theirs)
            .await
            .unwrap();
        let mine = file(&f.pool, &f.student).await;
        let draft = draft_naming(&f, mine).await;
        let refused = usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: draft,
                assignment_id: f.assignment,
            },
            &[theirs],
        )
        .await;
        assert!(matches!(refused, Err(FileError::Unusable { id }) if id == theirs));
        // The same through the draft: nothing is saved and the file stays the classmate's.
        let error = save_draft_naming(&f, &f.student, f.assignment, theirs)
            .await
            .unwrap_err();
        assert!(matches!(error, AssessmentError::Document(_)), "{error:?}");
        assert_eq!(
            holders(&f.pool, theirs).await,
            rows_of(&f.pool, f.assignment, &f.classmate).await
        );

        // The student's own file from a submission to a different assignment.
        let mut tx = f.pool.begin().await.unwrap();
        let module = content::create_module(
            &mut tx,
            &f.owner,
            f.course,
            ContentTitle::parse("M2").unwrap(),
            None,
        )
        .await
        .unwrap();
        content::publish_module(&mut tx, &f.owner, module.id, true)
            .await
            .unwrap();
        let other = content::create_item(
            &mut tx,
            &f.owner,
            rules(),
            module.id,
            ContentTitle::parse("Other").unwrap(),
            None,
            Some(Assignment {
                assignment_type: BuiltinType::Text,
                config: Map::new(),
                max_points: MaxPoints::parse("10").unwrap(),
                is_bonus: false,
                settings: Default::default(),
                class_on: None,
            }),
        )
        .await
        .unwrap();
        content::publish_item(&mut tx, &f.owner, other.id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let elsewhere = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, other.id, elsewhere)
            .await
            .unwrap();
        let refused = usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: draft,
                assignment_id: f.assignment,
            },
            &[elsewhere],
        )
        .await;
        assert!(matches!(refused, Err(FileError::Unusable { id }) if id == elsewhere));
    })
    .await;
}

#[tokio::test]
async fn a_file_held_by_an_item_is_not_usable_by_a_draft_even_next_to_an_own_attempt() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, f.assignment, id)
            .await
            .unwrap();
        submit(&f).await;
        let draft = {
            // A new draft row that names nothing yet.
            let other = file(&f.pool, &f.student).await;
            save_draft_naming(&f, &f.student, f.assignment, other)
                .await
                .unwrap();
            *rows_of(&f.pool, f.assignment, &f.student)
                .await
                .last()
                .unwrap()
        };
        usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: draft,
                assignment_id: f.assignment,
            },
            &[id],
        )
        .await
        .unwrap();
        // An item now holds the file as well: the earlier attempt no longer makes it the student's.
        item_names(&f.pool, f.lesson, false, id).await;
        let refused = usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: draft,
                assignment_id: f.assignment,
            },
            &[id],
        )
        .await;
        assert!(matches!(refused, Err(FileError::Unusable { id: bad }) if bad == id));
        let only_item = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, true, only_item).await;
        let refused = usable(
            &f,
            &f.student,
            UsageOwner::Submission {
                id: draft,
                assignment_id: f.assignment,
            },
            &[only_item],
        )
        .await;
        assert!(matches!(refused, Err(FileError::Unusable { id: bad }) if bad == only_item));
    })
    .await;
}

#[tokio::test]
async fn upload_is_authorized_by_the_owner() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let mut conn = f.pool.acquire().await.unwrap();
        let item = Owner::Item(f.lesson);
        let assignment = Owner::Assignment(f.assignment);
        authorize_upload(&mut conn, &f.owner, &item, Utc::now())
            .await
            .unwrap();
        authorize_upload(&mut conn, &f.student, &assignment, Utc::now())
            .await
            .unwrap();
        // A student may see the lesson but not change it.
        let error = authorize_upload(&mut conn, &f.student, &item, Utc::now()).await;
        assert!(matches!(
            error,
            Err(FileError::Forbidden(AccessError::Denied))
        ));
        // Staff do not submit work.
        let error = authorize_upload(&mut conn, &f.owner, &assignment, Utc::now()).await;
        assert!(matches!(
            error,
            Err(FileError::Forbidden(AccessError::Denied))
        ));
        // A course the actor is not on, and an id that is no item.
        for (actor, owner) in [
            (&f.stranger, item),
            (&f.stranger, assignment),
            (&f.owner, Owner::Item(Uuid::now_v7())),
            (&f.student, Owner::Assignment(Uuid::now_v7())),
        ] {
            let error = authorize_upload(&mut conn, actor, &owner, Utc::now()).await;
            assert!(matches!(error, Err(FileError::NotFound)), "{owner:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn upload_to_an_archived_course_is_a_conflict() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let mut conn = f.pool.acquire().await.unwrap();
        teaching::archive_course(&mut conn, &f.owner, f.course)
            .await
            .unwrap();
        for (actor, owner) in [
            (&f.owner, Owner::Item(f.lesson)),
            (&f.student, Owner::Assignment(f.assignment)),
        ] {
            let error = authorize_upload(&mut conn, actor, &owner, Utc::now()).await;
            assert!(matches!(error, Err(FileError::Conflict(_))), "{error:?}");
        }
    })
    .await;
}

fn incoming(id: Uuid) -> crate::files::routes::Incoming<'static> {
    crate::files::routes::Incoming {
        id,
        data: axum::body::Bytes::from_static(b"hello"),
        name: "a.txt",
        claimed: "text/plain",
    }
}

/// The upload is authorized, then the course is archived while the body is on its way: the row is
/// refused, and no object stays behind (the sweep only walks rows).
#[tokio::test]
async fn an_upload_whose_row_is_refused_leaves_no_object() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let owner = Owner::Item(f.lesson);
        let mut conn = f.pool.acquire().await.unwrap();
        authorize_upload(&mut conn, &f.owner, &owner, Utc::now())
            .await
            .unwrap();
        teaching::archive_course(&mut conn, &f.owner, f.course)
            .await
            .unwrap();
        drop(conn);
        let id = Uuid::now_v7();
        let error = crate::files::routes::store_upload(
            &f.pool,
            &storage,
            1024,
            &f.owner,
            &owner,
            incoming(id),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, FileError::Conflict(_)), "{error:?}");
        assert!(gone(&f.pool, &storage, id).await);
    })
    .await;
}

/// The success path keeps both the row and the object.
#[tokio::test]
async fn a_registered_upload_keeps_its_object() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = Uuid::now_v7();
        let file = crate::files::routes::store_upload(
            &f.pool,
            &storage,
            1024,
            &f.owner,
            &Owner::Item(f.lesson),
            incoming(id),
        )
        .await
        .unwrap();
        assert_eq!((file.id, file.mime.as_str()), (id, "text/plain"));
        assert!(kept(&f.pool, &storage, id).await);
    })
    .await;
}

fn memory() -> Storage {
    Storage::from_store(Arc::new(InMemory::new()))
}

/// A file with bytes in `storage` and a row, uploaded by `uploader`, referenced by nobody.
async fn stored_file(
    pool: &PgPool,
    storage: &Storage,
    uploader: &Actor,
    data: &'static [u8],
) -> Uuid {
    let id = Uuid::now_v7();
    let stored = files::put(storage, id, data, 1024).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    files::insert(
        &mut conn,
        &stored,
        uploader.user_id(),
        "a.txt",
        "text/plain",
    )
    .await
    .unwrap();
    id
}

/// Whether the row and the object of `id` are both still there.
async fn kept(pool: &PgPool, storage: &Storage, id: Uuid) -> bool {
    let mut conn = pool.acquire().await.unwrap();
    let Some(file) = files::get(&mut conn, id).await.unwrap() else {
        return false;
    };
    files::read(storage, &file, None).await.is_ok()
}

/// Whether neither the row nor the object of `id` is left.
async fn gone(pool: &PgPool, storage: &Storage, id: Uuid) -> bool {
    let mut conn = pool.acquire().await.unwrap();
    let row = files::get(&mut conn, id).await.unwrap();
    let object = files::read(
        storage,
        &files::File {
            id,
            uploaded_by: Uuid::nil(),
            name: String::new(),
            mime: String::new(),
            size: 0,
            sha256: String::new(),
            created_at: Utc::now(),
        },
        None,
    )
    .await;
    row.is_none() && object.is_err()
}

#[tokio::test]
async fn sweep_deletes_unreferenced_files_older_than_24h() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = stored_file(&f.pool, &storage, &f.owner, b"abc").await;
        let later = Utc::now() + Duration::hours(25);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 1);
        assert!(gone(&f.pool, &storage, id).await);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
    })
    .await;
}

#[tokio::test]
async fn sweep_keeps_unreferenced_files_younger_than_24h() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = stored_file(&f.pool, &storage, &f.owner, b"abc").await;
        let almost = Utc::now() + Duration::hours(23);
        assert_eq!(sweep(&f.pool, &storage, almost).await.unwrap(), 0);
        assert!(kept(&f.pool, &storage, id).await);
    })
    .await;
}

#[tokio::test]
async fn sweep_keeps_a_file_referenced_by_an_item_working_or_published_row() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let working = stored_file(&f.pool, &storage, &f.owner, b"w").await;
        let published = stored_file(&f.pool, &storage, &f.owner, b"p").await;
        let orphan = stored_file(&f.pool, &storage, &f.owner, b"o").await;
        item_names(&f.pool, f.lesson, false, working).await;
        item_names(&f.pool, f.lesson, true, published).await;
        let later = Utc::now() + Duration::hours(25);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 1);
        assert!(kept(&f.pool, &storage, working).await);
        assert!(kept(&f.pool, &storage, published).await);
        assert!(gone(&f.pool, &storage, orphan).await);
    })
    .await;
}

#[tokio::test]
async fn sweep_keeps_a_file_referenced_by_a_submission() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = stored_file(&f.pool, &storage, &f.student, b"abc").await;
        draft_naming(&f, id).await;
        let later = Utc::now() + Duration::hours(25);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
        assert!(kept(&f.pool, &storage, id).await);
        submit(&f).await;
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
        assert!(kept(&f.pool, &storage, id).await);
    })
    .await;
}

#[tokio::test]
async fn a_file_removed_from_every_reference_is_swept_after_24h() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let on_item = stored_file(&f.pool, &storage, &f.owner, b"i").await;
        let on_draft = stored_file(&f.pool, &storage, &f.student, b"d").await;
        item_names(&f.pool, f.lesson, false, on_item).await;
        let submission = draft_naming(&f, on_draft).await;
        let later = Utc::now() + Duration::hours(25);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);

        let mut conn = f.pool.acquire().await.unwrap();
        content::replace_item_files(&mut conn, f.lesson, false, &[])
            .await
            .unwrap();
        assessment::replace_submission_files(&mut conn, submission, &[])
            .await
            .unwrap();
        drop(conn);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 2);
        assert!(gone(&f.pool, &storage, on_item).await);
        assert!(gone(&f.pool, &storage, on_draft).await);
    })
    .await;
}

#[tokio::test]
async fn the_same_bytes_uploaded_twice_make_two_files_and_the_unused_one_is_swept() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let first = stored_file(&f.pool, &storage, &f.student, b"same").await;
        let second = stored_file(&f.pool, &storage, &f.student, b"same").await;
        assert_ne!(first, second);
        draft_naming(&f, second).await;
        let later = Utc::now() + Duration::hours(25);
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 1);
        assert!(gone(&f.pool, &storage, first).await);
        assert!(kept(&f.pool, &storage, second).await);
    })
    .await;
}
