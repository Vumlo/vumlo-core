//! `vumlo-cli admin create`: the first administrator of an installation, or another one.

use std::io::{IsTerminal, Read};

use anyhow::{Context, bail};
use db::identity::{Email, ModelError, Password, PasswordPolicy, PersonName};
use serde_json::json;
use uuid::Uuid;
use vumlo_config::Options;

use crate::runtime;

pub struct CreateAdmin {
    pub email: String,
    pub name: String,
    pub surname: String,
    pub password_stdin: bool,
    pub dry_run: bool,
}

pub async fn create(options: &Options, args: CreateAdmin) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let email = Email::parse(&args.email).context("invalid email")?;
    let name = PersonName::name(&args.name).context("invalid name")?;
    let surname = PersonName::surname(&args.surname).context("invalid surname")?;

    let pool = runtime::connect(settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;

    let existing = sqlx::query!(
        r#"SELECT id, role AS "role!", status AS "status!"
           FROM identity.users WHERE email = $1"#,
        email.as_str()
    )
    .fetch_optional(&pool)
    .await?;
    if let Some(user) = existing {
        if user.role != "admin" {
            bail!(
                "a user with this email already exists with the role {}; nothing was changed",
                user.role
            );
        }
        println!(
            "an admin with this email already exists ({}, {}); nothing was changed, the password \
             was not touched",
            user.id, user.status
        );
        return Ok(());
    }

    if args.dry_run && !args.password_stdin {
        println!(
            "would create the admin {} {}; the password would be asked for",
            name.as_str(),
            surname.as_str()
        );
        return Ok(());
    }

    let raw = read_password(args.password_stdin)?;
    let policy = PasswordPolicy::new(
        settings.auth.password_min_length,
        email.as_str(),
        name.as_str(),
        surname.as_str(),
        &settings.institution.name,
    );
    let password = Password::parse(raw, &policy).map_err(|e| match e {
        ModelError::Password(reasons) => {
            anyhow::anyhow!("password rejected: {}", reasons.join("; "))
        }
        other => anyhow::anyhow!("{other}"),
    })?;
    if args.dry_run {
        println!(
            "would create the admin {} {}",
            name.as_str(),
            surname.as_str()
        );
        return Ok(());
    }

    // Hash before the transaction opens (ADR-0016).
    let plain = password.expose().to_owned();
    let hash = tokio::task::spawn_blocking(move || db::identity::hash(&plain))
        .await
        .context("the hashing task failed")?
        .map_err(|e| anyhow::anyhow!("cannot hash the password: {e}"))?;

    let id = Uuid::now_v7();
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    db::record_mode(&mut tx, settings.institution.mode).await?;
    let inserted = sqlx::query!(
        "INSERT INTO identity.users (id, email, name, surname, role, must_change_password, email_verified_at)
         VALUES ($1, $2, $3, $4, 'admin', false, now())",
        id,
        email.as_str(),
        name.as_str(),
        surname.as_str()
    )
    .execute(&mut *tx)
    .await;
    if let Err(sqlx::Error::Database(e)) = &inserted
        && e.is_unique_violation()
    {
        bail!("a user with this email already exists; nothing was changed");
    }
    inserted?;
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'password', $3, $4)",
        Uuid::now_v7(),
        id,
        id.to_string(),
        hash.as_str()
    )
    .execute(&mut *tx)
    .await?;
    // ADR-0034: ids, roles and field names only; no email, no name, no password.
    db::insert_cli_audit(&mut tx, "user.create", "user", id, json!({"role": "admin"})).await?;
    tx.commit().await.context("cannot commit")?;

    println!("created the admin {id}");
    Ok(())
}

/// The password from a prompt (asked twice) or from standard input. Never from an argument.
fn read_password(from_stdin: bool) -> anyhow::Result<String> {
    if from_stdin {
        let mut text = String::new();
        std::io::stdin()
            .lock()
            .take(1024)
            .read_to_string(&mut text)
            .context("cannot read the password from standard input")?;
        let password = text.lines().next().unwrap_or_default().to_owned();
        if password.is_empty() {
            bail!("standard input held no password");
        }
        return Ok(password);
    }
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to ask for the password: pass --password-stdin and pipe it in");
    }
    let first = rpassword::prompt_password("Password: ").context("cannot read the password")?;
    let again =
        rpassword::prompt_password("Repeat the password: ").context("cannot read the password")?;
    if first != again {
        bail!("the two passwords differ");
    }
    Ok(first)
}
