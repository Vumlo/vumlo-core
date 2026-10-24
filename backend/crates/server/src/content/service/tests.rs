use crate::test_support::db_test;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Map;

use std::sync::LazyLock;

use super::*;
use crate::error::ApiError;
use crate::identity::{self, Email, NewUser, PersonName};
use crate::teaching::CourseTitle;
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
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let user = identity::create_user_unaudited_for_tests(
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
        let owner = Actor::new(user.id, Role::Teacher, Uuid::now_v7());
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

use std::time::Duration;

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

fn bonus(is_bonus: bool) -> ItemPatch {
    ItemPatch {
        is_bonus: Some(is_bonus),
        ..ItemPatch::default()
    }
}

struct Course {
    pool: sqlx::PgPool,
    owner: Actor,
    module: Uuid,
}

async fn course(pool: sqlx::PgPool) -> Course {
    let hash = identity::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = identity::create_user_unaudited_for_tests(
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
    let owner = Actor::new(user.id, Role::Teacher, Uuid::now_v7());
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

    /// Runs `patch` in its own transaction on a task and commits it when it succeeds.
    fn patch_in_background(
        &self,
        id: Uuid,
        patch: ItemPatch,
    ) -> tokio::task::JoinHandle<Result<Item, ContentError>> {
        let (pool, owner) = (self.pool.clone(), self.owner.clone());
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = update_item(&mut tx, &owner, rules(), id, patch).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    async fn assignment(&self, id: Uuid) -> Assignment {
        let mut conn = self.pool.acquire().await.unwrap();
        repo::find_assignment(&mut conn, id).await.unwrap().unwrap()
    }

    async fn regular_points(&self) -> String {
        let mut conn = self.pool.acquire().await.unwrap();
        let course_id = repo::find_module(&mut conn, self.module)
            .await
            .unwrap()
            .unwrap()
            .module
            .course_id;
        repo::regular_points(&mut conn, course_id, None)
            .await
            .unwrap()
            .to_string()
    }
}

/// Returns once a session waits for an advisory lock. Fails if the writer finished instead
/// (it did not wait for the lock) or if nobody waits within the timeout.
async fn wait_until_blocked<T>(pool: &sqlx::PgPool, writer: &tokio::task::JoinHandle<T>) {
    let mut conn = pool.acquire().await.unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !writer.is_finished() && repo::advisory_lock_waiters(&mut conn).await.unwrap() < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(waited.is_ok(), "no writer waited for the points lock");
    assert!(
        !writer.is_finished(),
        "the second writer finished without waiting for the points lock"
    );
}

fn points_limit(result: Result<Item, ContentError>) -> (&'static str, String) {
    match result {
        Err(ContentError::PointsLimit { field, message }) => (field, message),
        other => panic!("expected the points limit, got {other:?}"),
    }
}

#[tokio::test]
async fn two_creates_of_sixty_regular_points_one_gets_422() {
    db_test(async |pool| {
        let course = course(pool).await;

        let mut first = course.pool.begin().await.unwrap();
        course.create(&mut first, "60", false).await.unwrap();

        let second = tokio::spawn({
            let (pool, owner, module) = (course.pool.clone(), course.owner.clone(), course.module);
            async move {
                let mut tx = pool.begin().await.unwrap();
                let result = create_item(
                    &mut tx,
                    &owner,
                    rules(),
                    module,
                    ContentTitle::parse("B").unwrap(),
                    None,
                    Some(text("60", false)),
                )
                .await;
                if result.is_ok() {
                    tx.commit().await.unwrap();
                }
                result
            }
        });
        wait_until_blocked(&course.pool, &second).await;
        first.commit().await.unwrap();

        let error = second.await.unwrap();
        let (field, message) = points_limit(error);
        assert_eq!(field, "max_points");
        assert!(message.contains("40.00 are still free"), "{message}");
        assert_eq!(course.regular_points().await, "60.00");
    })
    .await;
}

#[tokio::test]
async fn a_patch_that_looked_like_a_lowering_is_checked_again_after_the_wait() {
    db_test(async |pool| {
        let course = course(pool).await;
        let x = course.committed("40", false).await;
        let y = course.committed("60", false).await;
        assert_ne!(x, y);

        // The first writer lowers X to 10 and fills the freed points with a new assignment.
        let mut first = course.pool.begin().await.unwrap();
        course.patch(&mut first, x, points("10")).await.unwrap();
        course.create(&mut first, "30", false).await.unwrap();

        // The second writer loads X (40) to authorize, then waits for the lock. Against the 40 it
        // read, 30 is a lowering; against the 10 that is committed by then, it is a raise.
        let second = course.patch_in_background(x, points("30"));
        wait_until_blocked(&course.pool, &second).await;
        first.commit().await.unwrap();

        let (field, message) = points_limit(second.await.unwrap());
        assert_eq!(field, "max_points");
        assert!(message.contains("10.00 are still free"), "{message}");
        assert_eq!(course.assignment(x).await.max_points.to_string(), "10.00");
        assert_eq!(course.regular_points().await, "100.00");
    })
    .await;
}

#[tokio::test]
async fn a_patch_that_looked_like_no_change_is_checked_again_after_the_wait() {
    db_test(async |pool| {
        let course = course(pool).await;
        let x = course.committed("50", false).await;
        course.committed("50", false).await;

        // The first writer makes X a bonus assignment and fills the freed points.
        let mut first = course.pool.begin().await.unwrap();
        course.patch(&mut first, x, bonus(true)).await.unwrap();
        course.create(&mut first, "50", false).await.unwrap();

        // The second writer loads X as a regular assignment, so `is_bonus = false` looks like no
        // change. Once the first commits, X is a bonus one and this makes it regular again.
        let second = course.patch_in_background(x, bonus(false));
        wait_until_blocked(&course.pool, &second).await;
        first.commit().await.unwrap();

        let (field, message) = points_limit(second.await.unwrap());
        assert_eq!(field, "is_bonus");
        assert!(message.contains("0.00 are still free"), "{message}");
        assert!(course.assignment(x).await.is_bonus);
        assert_eq!(course.regular_points().await, "100.00");
    })
    .await;
}
