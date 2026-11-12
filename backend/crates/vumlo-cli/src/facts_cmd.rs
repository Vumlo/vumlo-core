//! `vumlo-cli facts rebuild` (ADR-0093): makes the facts the access rules read again from the
//! submissions and the published grades, for one course or for everything. The students' own marks
//! stay. It changes the database, so it writes an audit row (no actor, source `cli`) in the same
//! transaction. It holds the facts table while it runs, so writers wait for it.

use anyhow::Context;
use serde_json::json;
use uuid::Uuid;
use vumlo_config::Options;

use crate::runtime;

/// `facts rebuild [--course <id>]`.
pub async fn rebuild(options: &Options, course: Option<Uuid>) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = runtime::connect(&loaded.settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    learning::rebuild_facts(&mut tx, course).await?;
    db::insert_cli_audit(
        &mut tx,
        "facts.rebuild",
        "content_facts",
        course.unwrap_or_else(Uuid::nil),
        json!({ "course_id": course }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    match course {
        Some(course) => println!("rebuilt the facts of course {course}"),
        None => println!("rebuilt the facts of every course"),
    }
    Ok(())
}
