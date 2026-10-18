//! `vumlo-cli db status` and `db migrate`.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use db::Status;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::Options;

use crate::runtime;

/// The state of the core migrations and of every compiled-in module's.
async fn states(pool: &PgPool) -> anyhow::Result<Vec<Status>> {
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let mut found = Vec::new();
    let all = std::iter::once(("core", &db::MIGRATOR)).chain(runtime::migrators());
    for (label, migrator) in all {
        found.push(db::status(&mut conn, label, migrator).await?);
    }
    Ok(found)
}

pub async fn status(options: &Options, as_json: bool) -> anyhow::Result<bool> {
    let loaded = runtime::load(options)?;
    let pool = runtime::connect(&loaded.settings).await?;
    let states = states(&pool).await?;
    let mut conn = pool.acquire().await?;
    let migrated = db::migrated_modules(&mut conn).await?;
    let known: Vec<&str> = states.iter().map(|s| s.label.as_str()).collect();
    let kept: Vec<&String> = migrated
        .iter()
        .filter(|name| !known.contains(&name.as_str()))
        .collect();
    let healthy = states
        .iter()
        .all(|s| s.newer.is_empty() && s.dirty.is_none());
    if as_json {
        let rows: Vec<_> = states
            .iter()
            .map(|s| {
                json!({
                    "label": s.label, "table": s.table, "applied": s.applied.len(),
                    "latest": s.latest, "pending": s.pending, "newer": s.newer, "dirty": s.dirty,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"migrations": rows, "kept_modules": kept}))?
        );
        return Ok(healthy);
    }
    for s in &states {
        println!(
            "{:<10} applied {:<3} latest {:<3} pending {}",
            s.label,
            s.applied.len(),
            s.latest.map_or("-".to_owned(), |v| v.to_string()),
            if s.pending.is_empty() {
                "none".to_owned()
            } else {
                format!("{:?}", s.pending)
            }
        );
        if !s.newer.is_empty() {
            println!(
                "           the database has migration(s) {:?} that this binary does not contain: \
                 it was migrated by a newer release",
                s.newer
            );
        }
        if let Some(version) = s.dirty {
            println!("           migration {version} started and did not finish");
        }
    }
    for name in kept {
        println!(
            "{name:<10} not compiled in; its tables and data are kept (_sqlx_migrations_{name})"
        );
    }
    Ok(healthy)
}

/// Applies what is pending: the core first, then the compiled-in modules. Refuses a database that
/// is newer than this binary. Writes one audit row when it applied anything.
pub async fn migrate(options: &Options, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let pool = runtime::connect(settings).await?;
    let states = states(&pool).await?;
    for s in &states {
        if !s.newer.is_empty() {
            bail!(
                "the {} schema is newer than this binary: the database has migration(s) {:?} that \
                 this release does not contain; update vumlo-cli instead of migrating",
                s.label,
                s.newer
            );
        }
        if let Some(version) = s.dirty {
            bail!(
                "migration {version} of {} started and did not finish; repair the database by \
                 hand (restore the last backup or fix and clear the dirty row)",
                s.label
            );
        }
    }
    if dry_run {
        for s in &states {
            if s.pending.is_empty() {
                println!("{:<10} up to date", s.label);
            } else {
                println!("{:<10} would apply {:?}", s.label, s.pending);
            }
        }
        return Ok(());
    }

    let mut applied: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let all = std::iter::once(("core", &db::MIGRATOR)).chain(runtime::migrators());
    for (label, migrator) in all {
        let versions = db::migrate(&pool, label, migrator)
            .await
            .with_context(|| format!("the {label} migrations failed"))?;
        if !versions.is_empty() {
            applied.insert(label.to_owned(), versions);
        }
    }

    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    db::record_mode(&mut tx, settings.institution.mode).await?;
    if !applied.is_empty() {
        db::insert_cli_audit(
            &mut tx,
            "db.migrate",
            "database",
            Uuid::nil(),
            json!({"applied": applied}),
        )
        .await?;
    }
    tx.commit().await.context("cannot commit")?;

    if applied.is_empty() {
        println!("the database is up to date");
    }
    for (label, versions) in &applied {
        println!("{label:<10} applied {} migration(s)", versions.len());
    }
    Ok(())
}
