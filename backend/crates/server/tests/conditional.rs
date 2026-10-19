//! `ETag` and `If-None-Match` on `GET` (ADR-0059).

mod common;

use common::{Env, db_test};
use serde_json::json;
use server::identity::Role;

#[tokio::test]
async fn an_answer_carries_a_tag_and_an_unchanged_one_is_a_304() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[]).await;
        let path = format!("/courses/{course}");

        let first = env.call("GET", &owner.token, &path, None).await;
        assert_eq!(first.status, 200);
        let tag = first.header("etag").to_owned();
        assert!(tag.starts_with("W/\""), "{tag}");
        assert_eq!(first.header("cache-control"), "private, no-cache");

        let again = env
            .http
            .get(&path)
            .bearer(&owner.token)
            .header("if-none-match", &tag)
            .send()
            .await;
        assert_eq!(again.status, 304);
        assert!(again.body.is_empty());
        assert_eq!(again.header("etag"), tag);

        // A list of tags, a strong spelling of the same tag, and `*` all name it.
        let strong = tag.trim_start_matches("W/");
        for presented in [
            format!("\"nope\", {tag}"),
            strong.to_owned(),
            "*".to_owned(),
        ] {
            let response = env
                .http
                .get(&path)
                .bearer(&owner.token)
                .header("if-none-match", &presented)
                .send()
                .await;
            assert_eq!(response.status, 304, "{presented}");
        }

        // The course changes: the old tag no longer matches, and the new answer has a new one.
        env.ok(
            "PATCH",
            &owner.token,
            &path,
            Some(json!({"title": "Renamed"})),
            200,
        )
        .await;
        let changed = env
            .http
            .get(&path)
            .bearer(&owner.token)
            .header("if-none-match", &tag)
            .send()
            .await;
        assert_eq!(changed.status, 200);
        assert_ne!(changed.header("etag"), tag);
        assert_eq!(changed.json()["title"], "Renamed");
    })
    .await;
}

#[tokio::test]
async fn a_tag_never_lets_one_person_read_what_another_may_not() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let stranger = env.user("stranger@x.edu", Role::Student).await;
        let (course, _) = env.course(&owner, &[]).await;
        let path = format!("/courses/{course}");
        let tag = env
            .call("GET", &owner.token, &path, None)
            .await
            .header("etag")
            .to_owned();

        // The stranger presents the owner's tag: the request is still authorized and answered on its own.
        let response = env
            .http
            .get(&path)
            .bearer(&stranger.token)
            .header("if-none-match", &tag)
            .send()
            .await;
        assert_eq!(response.status, 404);
        assert_eq!(response.headers.get("etag"), None);
    })
    .await;
}

#[tokio::test]
async fn errors_and_other_methods_are_left_alone() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let missing = env
            .call(
                "GET",
                &owner.token,
                "/courses/00000000-0000-7000-8000-000000000000",
                None,
            )
            .await;
        assert_eq!(missing.status, 404);
        assert_eq!(missing.headers.get("etag"), None);
        let created = env
            .call(
                "POST",
                &owner.token,
                "/courses",
                Some(json!({"title": "T"})),
            )
            .await;
        assert_eq!(created.status, 201);
        assert_eq!(created.headers.get("etag"), None);
        let login = env
            .http
            .post("/auth/login")
            .json(&json!({"email": "owner@x.edu", "password": common::PASSWORD, "client": "app"}))
            .send()
            .await;
        assert_eq!(
            login.header("cache-control"),
            "no-store",
            "a credential is not cacheable"
        );
    })
    .await;
}
