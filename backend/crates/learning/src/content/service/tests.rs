use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Map;
use test_support::db_test;

use std::sync::LazyLock;

use super::*;
use crate::teaching::CourseTitle;
use auth::{self, Email, NewUser, PersonName};
use common::ApiError;
use common::Role;

static PROFILES: LazyLock<EffectiveProfiles> = LazyLock::new(EffectiveProfiles::unrestricted);

/// The `hundred` scale and the profiles of the code, as most tests want them.
fn rules() -> ItemRules<'static> {
    ItemRules {
        scale: GradingScale::Hundred,
        profiles: &PROFILES,
    }
}

/// Two creators each read "the last position" before the other commits, so both pick the
/// same one. The unique constraint is deferred: neither insert waits for the other, and the
/// loser fails at its own commit. The steps run one after another on purpose, so the
/// interleaving is fixed by the code, not by timing.
#[tokio::test]
async fn two_creates_racing_for_the_last_position_one_gets_409() {
    db_test(async |pool| {
        let hash = auth::hash_password("irrelevant").await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let user = auth::create_user_unaudited_for_tests(
            &mut tx,
            NewUser {
                email: Email::parse("owner@x.edu").unwrap(),
                name: PersonName::name("Owner").unwrap(),
                surname: PersonName::surname("Owner").unwrap(),
                role: Role::Teacher,
            },
            hash,
        )
        .await
        .unwrap();
        let owner = Actor::for_tests(user.id, Role::Teacher, Uuid::now_v7());
        let course = teaching::create_course(
            &mut tx,
            &owner,
            CourseTitle::parse("Algebra").unwrap(),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let title = |text| ContentTitle::parse(text).unwrap();
        let mut winner = pool.begin().await.unwrap();
        let mut loser = pool.begin().await.unwrap();
        let first = create_module(&mut winner, &owner, course.id, title("First"), None)
            .await
            .unwrap();
        let second = create_module(&mut loser, &owner, course.id, title("Second"), None)
            .await
            .unwrap();
        assert_eq!((first.position, second.position), (1, 1));

        winner.commit().await.unwrap();
        let error = ContentError::from(loser.commit().await.expect_err("the position is taken"));
        assert!(matches!(error, ContentError::Conflict), "{error:?}");
        assert_eq!(
            ApiError::from(error).into_response().status(),
            StatusCode::CONFLICT
        );

        let mut conn = pool.acquire().await.unwrap();
        let ids = repo::module_ids(&mut conn, course.id, None).await.unwrap();
        assert_eq!(ids, [first.id], "only the winner is stored");

        // The client retries and now appends after the winner.
        let mut retry = pool.begin().await.unwrap();
        let again = create_module(&mut retry, &owner, course.id, title("Second"), None)
            .await
            .unwrap();
        retry.commit().await.unwrap();
        assert_eq!(again.position, 2);
    })
    .await;
}

// -----------------------------------------------------------------------------------------
// The points lock (ADR-0022). Each test holds the lock in one transaction, starts a second
// writer, waits until that writer is observed blocked on the lock, and only then lets the
// first one commit. Nothing sleeps: the order is forced by the lock, not by timing.
// -----------------------------------------------------------------------------------------

use crate::content::model::{BuiltinType, MaxPoints};

fn text(points: &str, is_bonus: bool) -> Assignment {
    Assignment {
        assignment_type: BuiltinType::Text,
        config: Map::new(),
        max_points: MaxPoints::parse(points).unwrap(),
        is_bonus,
        settings: Default::default(),
        class_on: None,
    }
}

fn points(text: &str) -> ItemPatch {
    ItemPatch {
        max_points: Some(MaxPoints::parse(text).unwrap()),
        ..ItemPatch::default()
    }
}

struct Course {
    pool: sqlx::PgPool,
    owner: Actor,
    module: Uuid,
}

async fn course(pool: sqlx::PgPool) -> Course {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = auth::create_user_unaudited_for_tests(
        &mut tx,
        NewUser {
            email: Email::parse("owner@x.edu").unwrap(),
            name: PersonName::name("Owner").unwrap(),
            surname: PersonName::surname("Owner").unwrap(),
            role: Role::Teacher,
        },
        hash,
    )
    .await
    .unwrap();
    let owner = Actor::for_tests(user.id, Role::Teacher, Uuid::now_v7());
    let course = teaching::create_course(
        &mut tx,
        &owner,
        CourseTitle::parse("Algebra").unwrap(),
        None,
    )
    .await
    .unwrap();
    let module = create_module(
        &mut tx,
        &owner,
        course.id,
        ContentTitle::parse("M").unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Course {
        pool,
        owner,
        module: module.id,
    }
}

impl Course {
    /// Creates an assignment in the given transaction on the `hundred` scale.
    async fn create(
        &self,
        conn: &mut PgConnection,
        points: &str,
        is_bonus: bool,
    ) -> Result<Item, ContentError> {
        create_item(
            conn,
            &self.owner,
            rules(),
            self.module,
            ContentTitle::parse("A").unwrap(),
            None,
            Some(text(points, is_bonus)),
        )
        .await
    }

    /// Creates and commits an assignment.
    async fn committed(&self, points: &str, is_bonus: bool) -> Uuid {
        let mut tx = self.pool.begin().await.unwrap();
        let item = self.create(&mut tx, points, is_bonus).await.unwrap();
        tx.commit().await.unwrap();
        item.id
    }

    async fn patch(
        &self,
        conn: &mut PgConnection,
        id: Uuid,
        patch: ItemPatch,
    ) -> Result<Item, ContentError> {
        update_item(conn, &self.owner, rules(), id, patch).await
    }

    async fn assignment(&self, id: Uuid) -> Assignment {
        let mut conn = self.pool.acquire().await.unwrap();
        repo::find_assignment(&mut conn, id).await.unwrap().unwrap()
    }
}

/// ADR-0089: the regular assignments of a course may add up to more than 100 points on the
/// `hundred` scale; the gradebook cuts the total at the target, not the content at creation.
#[tokio::test]
async fn the_regular_assignments_of_a_course_may_add_up_to_more_than_a_hundred() {
    db_test(async |pool| {
        let course = course(pool).await;
        let x = course.committed("60", false).await;
        let y = course.committed("60", false).await;
        assert_ne!(x, y);

        // Raising one of them past what is left of 100 is allowed too.
        let mut tx = course.pool.begin().await.unwrap();
        course.patch(&mut tx, x, points("90")).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(course.assignment(x).await.max_points.to_string(), "90.00");
    })
    .await;
}

async fn item_file_rows(pool: &sqlx::PgPool, item: Uuid) -> Vec<(Uuid, bool)> {
    let mut rows = sqlx::query!(
        "SELECT file_id, published FROM content.item_files WHERE item_id = $1",
        item
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (row.file_id, row.published))
    .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[tokio::test]
async fn item_files_are_replaced_per_copy_and_go_with_the_item_and_its_module() {
    db_test(async |pool| {
        let course = course(pool).await;
        let item = course.committed("10", false).await;
        let other = course.committed("10", false).await;
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut conn = course.pool.acquire().await.unwrap();
        // A repeated id is stored once.
        repo::replace_item_files(&mut conn, item, false, &[a, b, b])
            .await
            .unwrap();
        repo::replace_item_files(&mut conn, item, true, &[a])
            .await
            .unwrap();
        repo::replace_item_files(&mut conn, other, false, &[c])
            .await
            .unwrap();
        let mut expected = vec![(a, false), (b, false), (a, true)];
        expected.sort();
        assert_eq!(item_file_rows(&course.pool, item).await, expected);

        // Rewriting one copy leaves the other and the other items alone.
        repo::replace_item_files(&mut conn, item, false, &[c])
            .await
            .unwrap();
        let mut expected = vec![(c, false), (a, true)];
        expected.sort();
        assert_eq!(item_file_rows(&course.pool, item).await, expected);
        repo::replace_item_files(&mut conn, item, false, &[])
            .await
            .unwrap();
        assert_eq!(item_file_rows(&course.pool, item).await, [(a, true)]);

        delete_item(&mut conn, &course.owner, item).await.unwrap();
        assert!(item_file_rows(&course.pool, item).await.is_empty());
        assert_eq!(item_file_rows(&course.pool, other).await, [(c, false)]);

        delete_module(&mut conn, &course.owner, course.module)
            .await
            .unwrap();
        assert!(item_file_rows(&course.pool, other).await.is_empty());
    })
    .await;
}

fn body_of(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

#[test]
fn a_file_item_references_the_file_its_body_names() {
    let file = Uuid::now_v7();
    let body = body_of(json!({"file_id": file.to_string(), "description": {"v": 1, "blocks": []}}));
    assert_eq!(
        referenced_file_ids(ItemKind::File, &body, None).unwrap(),
        [file]
    );
}

#[test]
fn a_file_item_without_a_usable_file_id_references_nothing() {
    for body in [
        json!({}),
        json!({"description": {"v": 1, "blocks": []}}),
        json!({"file_id": "not-a-uuid"}),
        json!({"file_id": 7}),
        json!({"file_id": null}),
    ] {
        assert_eq!(
            referenced_file_ids(ItemKind::File, &body_of(body.clone()), None).unwrap(),
            Vec::<Uuid>::new(),
            "{body}"
        );
    }
}

#[test]
fn text_and_assignment_bodies_reference_the_files_their_blocks_name() {
    let (image, file) = (Uuid::now_v7(), Uuid::now_v7());
    let body = body_of(json!({"v": 1, "attachments": [], "blocks": [
        {"id": "a", "type": "image", "props": {"file_id": image.to_string()}},
        {"id": "b", "type": "file", "props": {"file_id": file.to_string()}},
        {"id": "c", "type": "image", "props": {"file_id": image.to_string()}},
    ]}));
    for kind in [ItemKind::Text, ItemKind::Assignment] {
        assert_eq!(
            referenced_file_ids(kind, &body, None).unwrap(),
            [image, file]
        );
    }
    // A body that is not a document is an error, never "no files".
    let stray = body_of(json!({"file_id": Uuid::now_v7().to_string()}));
    assert!(referenced_file_ids(ItemKind::Text, &stray, None).is_err());
}

#[test]
fn a_file_items_description_blocks_count_and_a_quiz_config_adds_its_images() {
    let (own, described, in_prompt) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let body = body_of(
        json!({"file_id": own.to_string(), "description": {"v": 1, "blocks": [
        {"id": "a", "type": "image", "props": {"file_id": described.to_string()}}]}}),
    );
    assert_eq!(
        referenced_file_ids(ItemKind::File, &body, None).unwrap(),
        [own, described]
    );

    let config = body_of(json!({"v": 1, "questions": [{
        "id": "q1", "type": "long_text", "points": "1",
        "prompt": {"v": 1, "blocks": [
            {"id": "a", "type": "image", "props": {"file_id": in_prompt.to_string()}}]}}]}));
    let text = body_of(json!({"v": 1, "blocks": []}));
    assert_eq!(
        referenced_file_ids(
            ItemKind::Assignment,
            &text,
            Some((BuiltinType::Quiz, &config))
        )
        .unwrap(),
        [in_prompt]
    );
    // The other types hold no document in their config.
    assert!(
        referenced_file_ids(
            ItemKind::Assignment,
            &text,
            Some((BuiltinType::Form, &config))
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn a_config_or_document_the_parser_cannot_read_is_an_error_not_no_files() {
    let text = body_of(json!({"v": 1, "blocks": []}));
    // A quiz config of the wrong shape.
    let shapeless = body_of(json!({"questions": "none"}));
    assert!(
        referenced_file_ids(
            ItemKind::Assignment,
            &text,
            Some((BuiltinType::Quiz, &shapeless))
        )
        .is_err()
    );
    // A prompt that is not a document.
    let bad_prompt = body_of(json!({"v": 1, "questions": [{
        "id": "q1", "type": "long_text", "points": "1", "prompt": {"blocks": 3}}]}));
    assert!(
        referenced_file_ids(
            ItemKind::Assignment,
            &text,
            Some((BuiltinType::Quiz, &bad_prompt))
        )
        .is_err()
    );
    // A file item whose description is not a document.
    let body = body_of(json!({"description": {"v": 2}}));
    assert!(referenced_file_ids(ItemKind::File, &body, None).is_err());
}

#[tokio::test]
async fn set_due_at_changes_the_deadline_and_nothing_else_and_only_for_staff() {
    db_test(async |pool| {
        let course = course(pool).await;
        let id = course.committed("20", false).await;
        let due = "2030-05-01T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let before = course.assignment(id).await;

        let mut tx = course.pool.begin().await.unwrap();
        set_due_at(&mut tx, &course.owner, id, Some(due))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let after = course.assignment(id).await;
        assert_eq!(after.settings.due_at, Some(due));
        assert_eq!(after.settings.on_deadline, Some(OnDeadline::KeepDraft));
        assert_eq!(after.settings.accept_late, Some(false));
        assert_eq!(after.max_points, before.max_points);
        assert_eq!(after.is_bonus, before.is_bonus);

        let mut tx = course.pool.begin().await.unwrap();
        set_due_at(&mut tx, &course.owner, id, None).await.unwrap();
        tx.commit().await.unwrap();
        let cleared = course.assignment(id).await.settings;
        assert_eq!(
            (cleared.due_at, cleared.on_deadline, cleared.accept_late),
            (None, None, None)
        );

        // Someone who is not on the course cannot see the item, let alone change it.
        let stranger = Actor::for_tests(Uuid::now_v7(), Role::Student, Uuid::now_v7());
        let mut tx = course.pool.begin().await.unwrap();
        assert!(set_due_at(&mut tx, &stranger, id, Some(due)).await.is_err());
    })
    .await;
}
