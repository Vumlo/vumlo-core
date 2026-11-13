//! The owners (items, submissions) are real rows made through the services; the references between
//! them and the files are written through the repo functions of `content` and `assessment`.
use std::sync::{Arc, LazyLock};

use crate::tests::{create_item, db_test};
use chrono::{Duration, Utc};
use serde_json::{Map, json};
use sqlx::PgPool;

use super::*;
use crate::files::sweep;
use crate::items;
use access::CourseRole;
use auth::{self, Email, NewUser, PersonName};
use common::Role;
use content::{
    Assignment, BuiltinType, ContentTitle, EffectiveProfiles, ItemPatch, ItemRules, MaxPoints,
};
use files::{Storage, Stored};
use object_store::memory::InMemory;
use teaching::{self, CourseTitle};
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
    module: Uuid,
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
    let assignment = create_item(
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
        module: module.id,
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
    let item = create_item(
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
        items::update_item(conn, owner, rules(), item.id, patch)
            .await
            .unwrap();
    }
    content::publish_item(conn, owner, item.id).await.unwrap()
}

/// A file row uploaded by `uploader`, referenced by nobody.
async fn file(pool: &PgPool, uploader: &Actor) -> Uuid {
    file_named(pool, uploader, "a.txt", "text/plain").await
}

/// A file row of the given name and stored type.
async fn file_named(pool: &PgPool, uploader: &Actor, name: &str, mime: &str) -> Uuid {
    let id = Uuid::now_v7();
    let stored = Stored {
        id,
        size: 3,
        sha256: "00".to_owned(),
        head: b"abc".to_vec(),
    };
    let mut conn = pool.acquire().await.unwrap();
    files::insert(&mut conn, &stored, uploader.user_id(), name, mime)
        .await
        .unwrap();
    id
}

/// The file goes away, row and all: nothing ties the reference rows to it.
async fn remove_file(pool: &PgPool, id: Uuid) {
    let mut conn = pool.acquire().await.unwrap();
    files::forget(&mut conn, id).await.unwrap();
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
    crate::tests::save_draft(
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
    crate::tests::submit(&mut conn, &f.student, f.assignment, &PROFILES, Utc::now())
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
        let draft = create_item(
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
        let draft_in_open = create_item(
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

fn plain(ids: &[Uuid]) -> Vec<content::FileRef> {
    ids.iter()
        .enumerate()
        .map(|(index, &id)| content::FileRef {
            id,
            path: format!("body.blocks[{index}]"),
            image: false,
        })
        .collect()
}

/// What `check` answers for `actor` naming `refs` on `owner`.
async fn verdict(
    f: &Fixture,
    actor: &Actor,
    owner: UsageOwner,
    refs: &[content::FileRef],
) -> files::FileVerdict {
    let mut conn = f.pool.acquire().await.unwrap();
    check(&mut conn, actor.user_id(), &owner, refs)
        .await
        .unwrap()
}

/// The first file `check` refuses, or `None` when the owner may name them all.
async fn refused(f: &Fixture, actor: &Actor, owner: UsageOwner, ids: &[Uuid]) -> Option<Uuid> {
    verdict(f, actor, owner, &plain(ids)).await.unusable
}

#[tokio::test]
async fn check_accepts_own_unreferenced_upload() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        assert_eq!(
            refused(&f, &f.owner, UsageOwner::Item(f.lesson), &[id]).await,
            None
        );
        assert_eq!(
            refused(&f, &f.owner, UsageOwner::Item(f.lesson), &[]).await,
            None
        );
        // Someone else's unreferenced upload is not theirs to name.
        assert_eq!(
            refused(&f, &f.stranger, UsageOwner::Item(f.lesson), &[id]).await,
            Some(id)
        );
    })
    .await;
}

#[tokio::test]
async fn check_accepts_file_already_on_this_owner() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, id).await;
        // Another editor of the same item may keep naming it.
        assert_eq!(
            refused(&f, &f.stranger, UsageOwner::Item(f.lesson), &[id]).await,
            None
        );
        let theirs = file(&f.pool, &f.student).await;
        let submission = draft_naming(&f, theirs).await;
        let owner = UsageOwner::Submission {
            id: submission,
            assignment_id: f.assignment,
        };
        assert_eq!(refused(&f, &f.student, owner, &[theirs]).await, None);
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
        assert_eq!(
            refused(&f, &f.owner, UsageOwner::Item(f.lesson), &[id]).await,
            None
        );
        assert_eq!(
            refused(&f, &f.owner, UsageOwner::Item(f.assignment), &[id]).await,
            None
        );
        // An owner that does not hold it still cannot borrow it, even as the uploader.
        let stranger_submission = UsageOwner::Submission {
            id: Uuid::now_v7(),
            assignment_id: f.assignment,
        };
        assert_eq!(
            refused(&f, &f.owner, stranger_submission, &[id]).await,
            Some(id)
        );
    })
    .await;
}

#[tokio::test]
async fn check_rejects_file_of_another_owner_and_unknown_id() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let mine = file(&f.pool, &f.student).await;
        let submission = draft_naming(&f, mine).await;
        // The student cannot hang the same file on a lesson, and a teacher cannot hang it on
        // another item or on the classmate's submission, though they would pass as uploader.
        assert_eq!(
            refused(&f, &f.student, UsageOwner::Item(f.lesson), &[mine]).await,
            Some(mine)
        );
        let other = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, false, other).await;
        assert_eq!(
            refused(&f, &f.owner, UsageOwner::Item(f.assignment), &[other]).await,
            Some(other)
        );
        let on_submission = UsageOwner::Submission {
            id: submission,
            assignment_id: f.assignment,
        };
        assert_eq!(
            refused(&f, &f.owner, on_submission, &[other]).await,
            Some(other)
        );
        // Unknown id; and the first bad id of a list is the one reported.
        let unknown = Uuid::now_v7();
        assert_eq!(
            refused(&f, &f.student, on_submission, &[mine, unknown]).await,
            Some(unknown)
        );
    })
    .await;
}

#[tokio::test]
async fn check_names_the_first_unusable_file_in_the_order_given() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let (first, second) = (
            file(&f.pool, &f.student).await,
            file(&f.pool, &f.student).await,
        );
        let (low, high) = (first.min(second), first.max(second));
        // Neither is the owner's: both are refused, and the answer follows the order of the
        // references, not the order of the file ids the locks were taken in.
        let owner = UsageOwner::Item(f.lesson);
        assert_eq!(refused(&f, &f.owner, owner, &[high, low]).await, Some(high));
        assert_eq!(refused(&f, &f.owner, owner, &[low, high]).await, Some(low));
    })
    .await;
}

#[tokio::test]
async fn check_names_a_non_image_and_returns_every_name() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let text = file(&f.pool, &f.owner).await; // "a.txt", text/plain
        let image = file_named(&f.pool, &f.owner, "p.png", "image/png").await;
        let unknown = Uuid::now_v7();
        let refs = |ids: &[(Uuid, bool)]| -> Vec<content::FileRef> {
            ids.iter()
                .enumerate()
                .map(|(index, &(id, image))| content::FileRef {
                    id,
                    path: format!("body.blocks[{index}]"),
                    image,
                })
                .collect()
        };

        let answer = verdict(
            &f,
            &f.owner,
            UsageOwner::Item(f.lesson),
            &refs(&[(image, true), (text, true)]),
        )
        .await;
        assert_eq!(answer.unusable, None);
        assert_eq!(answer.not_image, Some(text));
        assert_eq!(answer.names.get(&image).map(String::as_str), Some("p.png"));
        assert_eq!(answer.names.get(&text).map(String::as_str), Some("a.txt"));

        // An unknown id is unusable, and it has no name.
        let answer = verdict(
            &f,
            &f.owner,
            UsageOwner::Item(f.lesson),
            &refs(&[(unknown, true)]),
        )
        .await;
        assert_eq!(answer.unusable, Some(unknown));
        assert!(answer.names.is_empty());
    })
    .await;
}

/// A body whose one block, of type `block` (`file` or `image`), names `file`.
fn body_naming(block: &str, file: Uuid) -> serde_json::Value {
    let props = if block == "image" {
        json!({"file_id": file, "alt": "A picture"})
    } else {
        json!({"file_id": file})
    };
    json!({"v": 1, "attachments": [], "blocks": [{"id": "b", "type": block, "props": props}]})
}

const FIELD: &str = "body.blocks[0].props.file_id";

#[tokio::test]
async fn an_item_naming_a_file_of_another_owner_is_refused_with_its_field_path() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        // The student's upload: the teacher may not name it, and nothing is written.
        let theirs = file(&f.pool, &f.student).await;
        let mut conn = f.pool.acquire().await.unwrap();
        let input = content::NewItemInput::document(
            ContentTitle::parse("New").unwrap(),
            Some(body_naming("file", theirs)),
            None,
        );

        let created = items::create_item(&mut conn, &f.owner, rules(), f.module, input).await;
        assert!(
            matches!(&created, Err(crate::LearningError::Content(
                content::ContentError::FileUnusable { id, field }
            )) if *id == theirs && field == FIELD),
            "{created:?}"
        );
        let patch = ItemPatch {
            body: Some(body_naming("file", theirs)),
            ..ItemPatch::default()
        };
        let updated = items::update_item(&mut conn, &f.owner, rules(), f.lesson, patch).await;
        assert!(
            matches!(&updated, Err(crate::LearningError::Content(
                content::ContentError::FileUnusable { id, field }
            )) if *id == theirs && field == FIELD),
            "{updated:?}"
        );
        assert!(holders_of_items(&f.pool, theirs).await.is_empty());
    })
    .await;
}

async fn holders_of_items(pool: &PgPool, file: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar!(
        "SELECT item_id FROM content.item_files WHERE file_id = $1",
        file
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_verdict_that_does_not_cover_the_files_refuses_them() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        // The student's own upload, which `check` would accept for their draft.
        let id = file(&f.pool, &f.student).await;
        let mut conn = f.pool.acquire().await.unwrap();

        // A write finished without the check (the empty verdict) is refused, not attached.
        let creation = content::begin_create_item(
            &mut conn,
            &f.owner,
            rules(),
            f.module,
            content::NewItemInput::document(
                ContentTitle::parse("New").unwrap(),
                Some(body_naming("file", id)),
                None,
            ),
        )
        .await
        .unwrap();
        let created =
            content::finish_create_item(&mut conn, &f.owner, creation, &Default::default()).await;
        assert!(
            matches!(&created, Err(content::ContentError::FileUnusable { id: bad, .. }) if *bad == id),
            "{created:?}"
        );

        let save = assessment::begin_save_draft(
            &mut conn,
            &f.student,
            f.assignment,
            &body_naming("file", id),
            &PROFILES,
            Utc::now(),
        )
        .await
        .unwrap();
        let saved = assessment::finish_save_draft(&mut conn, save, &Default::default()).await;
        assert!(
            matches!(saved, Err(AssessmentError::Document(_))),
            "{saved:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_image_block_needs_an_image_file() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let text = file(&f.pool, &f.owner).await; // text/plain
        let png = file_named(&f.pool, &f.owner, "p.png", "image/png").await;
        let mut conn = f.pool.acquire().await.unwrap();
        let patch = |file| ItemPatch {
            body: Some(body_naming("image", file)),
            ..ItemPatch::default()
        };

        let refused = items::update_item(&mut conn, &f.owner, rules(), f.lesson, patch(text)).await;
        assert!(
            matches!(&refused, Err(crate::LearningError::Content(
                content::ContentError::NotAnImage { field }
            )) if field == FIELD),
            "{refused:?}"
        );
        items::update_item(&mut conn, &f.owner, rules(), f.lesson, patch(png))
            .await
            .unwrap();
        assert_eq!(holders_of_items(&f.pool, png).await, [f.lesson]);
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
    crate::tests::save_draft(
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
        let on_draft = UsageOwner::Submission {
            id: draft,
            assignment_id: f.assignment,
        };
        assert_eq!(
            refused(&f, &f.student, on_draft, &[theirs]).await,
            Some(theirs)
        );
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
        let other = second_assignment(&f).await;
        let elsewhere = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, other, elsewhere)
            .await
            .unwrap();
        assert_eq!(
            refused(&f, &f.student, on_draft, &[elsewhere]).await,
            Some(elsewhere)
        );
    })
    .await;
}

/// A published text assignment in a module of its own, in the course of the fixture.
async fn second_assignment(f: &Fixture) -> Uuid {
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
    let other = create_item(
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
    other.id
}

#[tokio::test]
async fn a_draft_cannot_take_a_file_another_draft_just_attached() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let other = second_assignment(&f).await;
        // One upload of the student, named by their drafts on two assignments.
        let id = file(&f.pool, &f.student).await;

        // The first save has checked the file and not committed yet.
        let mut first = f.pool.begin().await.unwrap();
        crate::tests::save_draft(
            &mut first,
            &f.student,
            f.assignment,
            &body_naming("file", id),
            &PROFILES,
            Utc::now(),
        )
        .await
        .unwrap();
        let waiting = tokio::spawn({
            let (pool, student) = (f.pool.clone(), f.student.clone());
            async move {
                let mut tx = pool.begin().await.unwrap();
                let saved = crate::tests::save_draft(
                    &mut tx,
                    &student,
                    other,
                    &body_naming("file", id),
                    &PROFILES,
                    Utc::now(),
                )
                .await;
                tx.commit().await.unwrap();
                saved
            }
        });
        test_support::wait_until_blocked(&f.pool, &waiting).await;
        first.commit().await.unwrap();

        // The second one waited, then saw the first one's reference and was refused.
        let refused = waiting.await.unwrap();
        assert!(
            matches!(refused, Err(AssessmentError::Document(_))),
            "{refused:?}"
        );
        assert_eq!(
            holders(&f.pool, id).await,
            rows_of(&f.pool, f.assignment, &f.student).await
        );
    })
    .await;
}

#[tokio::test]
async fn two_first_saves_of_one_draft_run_one_after_the_other() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let empty = json!({"v": 1, "attachments": [], "blocks": []});
        let now = Utc::now();

        // The first save finds no draft and has not written one yet.
        let mut first = f.pool.begin().await.unwrap();
        let save = assessment::begin_save_draft(
            &mut first,
            &f.student,
            f.assignment,
            &empty,
            &PROFILES,
            now,
        )
        .await
        .unwrap();
        let id = save.submission_id();

        // A second first save would otherwise take the file rows before the draft row, against a
        // submit that holds the draft row: it has to wait here, before any file is locked.
        let waiting = tokio::spawn({
            let (pool, student, empty) = (f.pool.clone(), f.student.clone(), empty.clone());
            let assignment = f.assignment;
            async move {
                let mut tx = pool.begin().await.unwrap();
                let save = assessment::begin_save_draft(
                    &mut tx, &student, assignment, &empty, &PROFILES, now,
                )
                .await
                .unwrap();
                tx.commit().await.unwrap();
                save.submission_id()
            }
        });
        test_support::wait_until_blocked(&f.pool, &waiting).await;
        assessment::finish_save_draft(&mut first, save, &::files::FileVerdict::default())
            .await
            .unwrap();
        first.commit().await.unwrap();

        // It then found the draft the first one wrote, and keeps its row.
        assert_eq!(waiting.await.unwrap(), id);
    })
    .await;
}

#[tokio::test]
async fn a_submit_checks_the_files_of_the_draft_again() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, f.assignment, id)
            .await
            .unwrap();
        // The file went away after the save (nothing ties its row to the draft).
        remove_file(&f.pool, id).await;

        let mut conn = f.pool.acquire().await.unwrap();
        let error =
            crate::tests::submit(&mut conn, &f.student, f.assignment, &PROFILES, Utc::now())
                .await
                .unwrap_err();

        assert!(matches!(error, AssessmentError::Document(_)), "{error:?}");
        assert!(error.refuses_submit());
        assert_eq!(rows_of(&f.pool, f.assignment, &f.student).await.len(), 1);
        let attempts = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM assessment.submissions
               WHERE assignment_id = $1 AND status = 'submitted'"#,
            f.assignment
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 0);
    })
    .await;
}

#[tokio::test]
async fn an_auto_submit_whose_file_is_gone_is_refused_and_leaves_the_draft() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let id = file(&f.pool, &f.student).await;
        save_draft_naming(&f, &f.student, f.assignment, id)
            .await
            .unwrap();
        // The deadline passes with the setting on, then the file goes away.
        let patch = ItemPatch {
            settings: content::SettingsPatch {
                due_at: Some(Some(Utc::now() - Duration::hours(1))),
                on_deadline: Some(Some("auto_submit".to_owned())),
                accept_late: Some(Some(false)),
                ..Default::default()
            },
            ..ItemPatch::default()
        };
        let mut conn = f.pool.acquire().await.unwrap();
        items::update_item(&mut conn, &f.owner, rules(), f.assignment, patch)
            .await
            .unwrap();
        drop(conn);
        remove_file(&f.pool, id).await;
        let draft = assessment::DueDraft {
            assignment_id: f.assignment,
            student_id: f.student.user_id(),
            course_id: f.course,
        };

        let mut conn = f.pool.acquire().await.unwrap();
        let error = crate::tests::auto_submit(&mut conn, &draft, &PROFILES, Utc::now())
            .await
            .unwrap_err();

        assert!(error.refuses_submit(), "{error:?}");
        assert_eq!(rows_of(&f.pool, f.assignment, &f.student).await.len(), 1);
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
        let on_draft = UsageOwner::Submission {
            id: draft,
            assignment_id: f.assignment,
        };
        assert_eq!(refused(&f, &f.student, on_draft, &[id]).await, None);
        // An item now holds the file as well: the earlier attempt no longer makes it the student's.
        item_names(&f.pool, f.lesson, false, id).await;
        assert_eq!(refused(&f, &f.student, on_draft, &[id]).await, Some(id));
        let only_item = file(&f.pool, &f.owner).await;
        item_names(&f.pool, f.lesson, true, only_item).await;
        assert_eq!(
            refused(&f, &f.student, on_draft, &[only_item]).await,
            Some(only_item)
        );
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

#[tokio::test]
async fn sweep_leaves_a_file_that_a_running_attach_has_locked() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = stored_file(&f.pool, &storage, &f.owner, b"abc").await;
        let later = Utc::now() + Duration::hours(25);

        // An attach that checked the file and has not committed yet holds this lock.
        let mut attach = f.pool.begin().await.unwrap();
        files::lock(&mut attach, &[id]).await.unwrap();

        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
        assert!(kept(&f.pool, &storage, id).await);

        // The attach gave up: the file is an orphan again and the next round takes it.
        attach.rollback().await.unwrap();
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 1);
        assert!(gone(&f.pool, &storage, id).await);
    })
    .await;
}

#[tokio::test]
async fn sweep_keeps_a_file_whose_attach_committed_while_the_sweep_skipped_it() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        let storage = memory();
        let id = stored_file(&f.pool, &storage, &f.owner, b"abc").await;
        let later = Utc::now() + Duration::hours(25);

        let mut attach = f.pool.begin().await.unwrap();
        files::lock(&mut attach, &[id]).await.unwrap();
        content::replace_item_files(&mut attach, f.lesson, false, &[id])
            .await
            .unwrap();
        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
        attach.commit().await.unwrap();

        assert_eq!(sweep(&f.pool, &storage, later).await.unwrap(), 0);
        assert!(kept(&f.pool, &storage, id).await);
    })
    .await;
}
