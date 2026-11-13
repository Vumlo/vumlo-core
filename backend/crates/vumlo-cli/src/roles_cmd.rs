//! `vumlo-cli roles show | set` (ADR-0094): what the unit roles `manager` and `viewer` may do. A change
//! writes one audit row (no actor, source `cli`) in the transaction that makes it; the same set again
//! says nothing was changed and writes nothing.

use access::{UnitAbility, UnitRole};
use anyhow::Context;
use serde_json::json;
use uuid::Uuid;
use vumlo_config::Options;

use crate::runtime;

async fn connect(options: &Options) -> anyhow::Result<sqlx::PgPool> {
    let loaded = runtime::load(options)?;
    let pool = runtime::connect(&loaded.settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    Ok(pool)
}

fn names(abilities: &[UnitAbility]) -> String {
    if abilities.is_empty() {
        "view only".to_owned()
    } else {
        abilities
            .iter()
            .map(|ability| ability.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub async fn show(options: &Options) -> anyhow::Result<()> {
    let pool = connect(options).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    println!(
        "Every unit role can always view its unit and the courses in it; these switches add to that."
    );
    for (role, abilities) in org::role_abilities(&mut conn).await? {
        println!("{:<8} {}", role.as_str(), names(&abilities));
    }
    Ok(())
}

/// `view_grades,manage_roster`, or `none` for no switch at all.
fn parse_abilities(text: &str) -> anyhow::Result<Vec<UnitAbility>> {
    if text.trim() == "none" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|name| {
            name.trim()
                .parse::<UnitAbility>()
                .map_err(anyhow::Error::msg)
        })
        .collect()
}

pub async fn set(
    options: &Options,
    role: &str,
    abilities: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    let role: UnitRole = role.parse().map_err(anyhow::Error::msg)?;
    let abilities = parse_abilities(abilities)?;
    let pool = connect(options).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let now = org::role_abilities(&mut tx)
        .await?
        .into_iter()
        .find(|(held, _)| *held == role)
        .map(|(_, abilities)| abilities)
        .unwrap_or_default();
    let stored = org::apply_role_abilities(&mut tx, role, abilities).await?;
    let current: Vec<&str> = now.iter().map(|ability| ability.as_str()).collect();
    if stored == current {
        println!(
            "{} has these switches already; nothing was changed",
            role.as_str()
        );
        return Ok(());
    }
    if dry_run {
        println!(
            "would set the switches of {} to: {}",
            role.as_str(),
            stored.join(", ")
        );
        return Ok(());
    }
    db::insert_cli_audit(
        &mut tx,
        "unit.abilities.set",
        "unit_role",
        // A role has no row of its own: the entry names the installation by the nil id.
        Uuid::nil(),
        json!({"role": role.as_str(), "abilities": stored}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("set the switches of {}", role.as_str());
    Ok(())
}
