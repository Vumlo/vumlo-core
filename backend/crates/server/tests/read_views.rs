//! What the read views between the modules of `learning` leave out (ADR-0093, migration 0054).
//! A reader above the owner sees these views and never the table, so three of the rules of
//! CLAUDE.md section 6 hold by construction: no draft grade, no content of a draft submission, no
//! item body. (The working title and a quiz's answer key are still there; the readers above strip
//! them in code.)

mod common;

use common::db_test;
use sqlx::PgPool;
use uuid::Uuid;

async fn user(pool: &PgPool, email: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO auth.users (id, email, name, surname, role)
         VALUES ($1, $2, 'Test', 'User', 'teacher')",
        id,
        email
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn columns(pool: &PgPool, schema: &str, view: &str) -> Vec<String> {
    sqlx::query_scalar!(
        r#"SELECT column_name AS "name!" FROM information_schema.columns
           WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position"#,
        schema,
        view
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_view_of_grades_has_published_columns_only_and_hides_a_row_with_nothing_published() {
    db_test(async |pool| {
        let student = user(&pool, "s@x.edu").await;
        let (draft_only, published) = (Uuid::now_v7(), Uuid::now_v7());
        sqlx::query!(
            "INSERT INTO gradebook.grades (assignment_id, student_id, draft_score, draft_updated_by)
             VALUES ($1, $2, 5, $2)",
            draft_only,
            student
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO gradebook.grades
                 (assignment_id, student_id, draft_score, draft_feedback, draft_updated_by,
                  published_score, published_feedback, published_by, published_at)
             VALUES ($1, $2, 9, 'draft words', $2, 7, 'published words', $2, now())",
            published,
            student
        )
        .execute(&pool)
        .await
        .unwrap();

        let seen = columns(&pool, "gradebook", "published_grades").await;
        assert!(
            seen.iter().all(|name| !name.starts_with("draft")),
            "a draft column is in the view: {seen:?}"
        );
        let rows = sqlx::query!(
            r#"SELECT assignment_id AS "assignment_id!", published_score
               FROM gradebook.published_grades"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "a grade with only a draft is not in the view"
        );
        assert_eq!(rows[0].assignment_id, published);
        assert_eq!(rows[0].published_score.unwrap().to_string(), "7.00");
    })
    .await;
}

#[tokio::test]
async fn the_views_of_submissions_show_no_draft_content_and_the_attempts_only_what_was_submitted() {
    db_test(async |pool| {
        let student = user(&pool, "s@x.edu").await;
        let assignment = Uuid::now_v7();
        for (status, attempt) in [("draft", None), ("submitted", Some(1))] {
            sqlx::query!(
                "INSERT INTO assessment.submissions
                     (id, assignment_id, student_id, status, attempt, content, submitted_at)
                 VALUES ($1, $2, $3, $4, $5, '{\"answers\": {}}',
                         CASE WHEN $4 = 'submitted' THEN now() END)",
                Uuid::now_v7(),
                assignment,
                student,
                status,
                attempt
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        assert!(
            !columns(&pool, "assessment", "submission_index")
                .await
                .contains(&"content".to_owned()),
            "the index of submissions carries no content"
        );
        let statuses = sqlx::query_scalar!(
            r#"SELECT status AS "status!" FROM assessment.submission_index ORDER BY status"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            statuses,
            ["draft", "submitted"],
            "the index knows both exist"
        );
        let attempts = sqlx::query_scalar!(
            r#"SELECT attempt AS "attempt!" FROM assessment.submitted_attempts"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(attempts, [1], "a draft is not an attempt");
    })
    .await;
}

#[tokio::test]
async fn the_view_of_items_leaves_out_the_text_and_says_when_a_published_item_was_edited() {
    db_test(async |pool| {
        let owner = user(&pool, "t@x.edu").await;
        let (course, module, item) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        sqlx::query!(
            "INSERT INTO teaching.courses (id, kind, title, owner_id, created_by)
             VALUES ($1, 'regular', 'Course', $2, $2)",
            course,
            owner
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO content.modules (id, course_id, position, title) VALUES ($1, $2, 1, 'M')",
            module,
            course
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO content.items (id, module_id, course_id, position, kind, title, status,
                                        published_title, published_body, last_published_at)
             VALUES ($1, $2, $3, 1, 'text', 'Lecture', 'published', 'Lecture',
                     '{\"v\": 1, \"blocks\": [], \"attachments\": []}', now())",
            item,
            module,
            course
        )
        .execute(&pool)
        .await
        .unwrap();

        let seen = columns(&pool, "content", "item_list").await;
        assert!(
            !seen.iter().any(|name| name.ends_with("body")),
            "the text of an item is in the view: {seen:?}"
        );
        let edited = |pool: PgPool| async move {
            sqlx::query_scalar!(
                r#"SELECT edited AS "edited!" FROM content.item_list WHERE id = $1"#,
                item
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        assert!(
            !edited(pool.clone()).await,
            "same title and body as published"
        );
        sqlx::query!(
            "UPDATE content.items SET title = 'Lecture 2' WHERE id = $1",
            item
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(edited(pool.clone()).await, "the working title differs");
    })
    .await;
}
