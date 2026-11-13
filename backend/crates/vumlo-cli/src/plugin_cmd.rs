//! `vumlo-cli plugin ...` (ADR-0066): install, list, grant, disable, enable and remove the external
//! services that receive events and call the plugin API. Every command that changes the database
//! writes an audit row (no actor, source `cli`) in the same transaction, and says nothing was
//! changed, writing nothing, when it changes nothing. The token and the secret of an install are
//! printed once, by `install`, and nowhere else. A secret setting is never printed at all.

use std::io::{BufRead, IsTerminal, Write};

use access::{CourseScope, Scope};
use anyhow::{Context, bail};
use plugins::{Install, InstallStatus, PluginManifest, SecretKey, SettingScope, SettingView};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{Options, Settings};

use crate::runtime;

pub struct InstallArgs {
    pub base_url: String,
    /// Comma-separated scopes to grant, or `none`. Absent: ask for each scope that was requested.
    pub grant: Option<String>,
    /// `all`, `none` or course ids. Absent: ask.
    pub courses: Option<String>,
    pub dry_run: bool,
}

fn courses_json(courses: &CourseScope) -> Value {
    match courses {
        CourseScope::All => json!("all"),
        CourseScope::Only(list) => json!(list),
    }
}

fn courses_text(courses: &CourseScope) -> String {
    match courses {
        CourseScope::All => "all courses".to_owned(),
        CourseScope::Only(list) if list.is_empty() => "no course".to_owned(),
        CourseScope::Only(list) => list
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// `all`, `none`, or course ids separated by commas.
fn parse_courses(text: &str) -> anyhow::Result<CourseScope> {
    match text.trim() {
        "all" => Ok(CourseScope::All),
        "none" => Ok(CourseScope::Only(Vec::new())),
        list => {
            let mut ids = list
                .split(',')
                .map(|id| {
                    id.trim()
                        .parse::<Uuid>()
                        .map_err(|_| anyhow::anyhow!("`{}` is not a course id", id.trim()))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            ids.sort();
            ids.dedup();
            Ok(CourseScope::Only(ids))
        }
    }
}

/// Asks which courses the plugin may reach. There is no default: an empty answer asks again, and
/// the end of the input is an error.
fn ask_courses() -> anyhow::Result<CourseScope> {
    loop {
        print!("courses the plugin may reach (`all`, `none`, or course ids, comma-separated): ");
        std::io::stdout()
            .flush()
            .context("cannot write the prompt")?;
        let mut line = String::new();
        let read = std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("cannot read the answer")?;
        if read == 0 {
            bail!("no answer: give --courses all, none, or course ids");
        }
        if line.trim().is_empty() {
            continue;
        }
        match parse_courses(&line) {
            Ok(courses) => return Ok(courses),
            Err(error) => println!("{error}"),
        }
    }
}

fn scope_list(scopes: &[Scope]) -> String {
    if scopes.is_empty() {
        "none".to_owned()
    } else {
        scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn scope_names(scopes: &[Scope]) -> Vec<&'static str> {
    scopes.iter().map(|scope| scope.as_str()).collect()
}

/// `a,b` or `none`.
fn parse_scopes(text: &str) -> anyhow::Result<Vec<Scope>> {
    if text.trim() == "none" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|name| name.trim().parse::<Scope>().map_err(anyhow::Error::msg))
        .collect()
}

/// Asks, for each scope the plugin requested, whether to grant it.
fn ask(manifest: &PluginManifest) -> anyhow::Result<Vec<Scope>> {
    ask_scopes(&manifest.scopes)
}

fn ask_scopes(scopes: &[Scope]) -> anyhow::Result<Vec<Scope>> {
    let mut granted = Vec::new();
    let stdin = std::io::stdin();
    for scope in scopes {
        print!("grant `{}`? [y/N] ", scope.as_str());
        std::io::stdout()
            .flush()
            .context("cannot write the prompt")?;
        let mut line = String::new();
        stdin
            .lock()
            .read_line(&mut line)
            .context("cannot read the answer")?;
        if matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            granted.push(*scope);
        }
    }
    Ok(granted)
}

async fn ready(settings: &Settings) -> anyhow::Result<PgPool> {
    let pool = runtime::connect(settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    Ok(pool)
}

fn key(settings: &Settings) -> anyhow::Result<SecretKey> {
    SecretKey::from_config(&settings.plugins.secret_key).context(
        "plugins.secret_key is not set: put VUMLO__PLUGINS__SECRET_KEY (32 characters or more, \
         for example from `openssl rand -base64 32`) in the environment",
    )
}

pub async fn install(options: &Options, args: InstallArgs) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let key = key(&loaded.settings)?;
    // The plugin is read before anything else: the answer decides what there is to approve.
    let manifest = plugins::fetch_manifest(&args.base_url).await?;
    println!("plugin {} {}", manifest.id, manifest.version);
    println!(
        "  subscribes to: {}",
        if manifest.subscribes.is_empty() {
            "nothing".to_owned()
        } else {
            manifest.subscribes.join(", ")
        }
    );
    println!("  asks for scopes: {}", scope_list(&manifest.scopes));
    if !manifest.settings.is_empty() {
        let keys: Vec<_> = manifest.settings.keys().map(String::as_str).collect();
        println!(
            "  declares settings: {} (see `vumlo-cli plugin settings list {}`)",
            keys.join(", "),
            manifest.id
        );
    }
    let granted = match &args.grant {
        Some(text) => parse_scopes(text)?,
        None => ask(&manifest)?,
    };
    if let Some(extra) = granted.iter().find(|s| !manifest.scopes.contains(s)) {
        bail!(
            "`{}` cannot be granted: the plugin did not ask for it",
            extra.as_str()
        );
    }
    let courses = match &args.courses {
        Some(text) => parse_courses(text)?,
        None => ask_courses()?,
    };
    if args.dry_run {
        println!(
            "would install {} with the scopes: {}; courses: {}",
            manifest.id,
            scope_list(&granted),
            courses_text(&courses)
        );
        return Ok(());
    }

    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let installed =
        plugins::install(&mut tx, &key, &args.base_url, &manifest, &granted, &courses).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.install",
        "plugin",
        installed.install.id,
        json!({
            "plugin_id": installed.install.plugin_id,
            "version": installed.install.version,
            "scopes": scope_names(&installed.install.scopes),
            "courses": courses_json(&installed.install.courses),
            "subscribes": installed.install.subscribes,
        }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;

    println!(
        "installed {} ({}) with the scopes: {}; courses: {}",
        installed.install.plugin_id,
        installed.install.id,
        scope_list(&installed.install.scopes),
        courses_text(&installed.install.courses)
    );
    println!(
        "These are shown once. Give them to the plugin, for example as environment variables:"
    );
    println!("  VUMLO_PLUGIN_TOKEN={}", installed.token.expose());
    println!("  VUMLO_WEBHOOK_SECRET={}", installed.secret);
    Ok(())
}

pub async fn list(options: &Options) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let all = plugins::list(&mut conn).await?;
    if all.is_empty() {
        println!("no plugins installed");
    }
    for one in all {
        let install = &one.install;
        println!(
            "{:<16} {:<10} {:<9} scopes: {} (asked for: {}); courses: {}; deliveries: {} pending, {} failed",
            install.plugin_id,
            install.version,
            install.status.as_str(),
            scope_list(&install.scopes),
            scope_list(&install.requested_scopes),
            courses_text(&install.courses),
            one.pending,
            one.failed,
        );
    }
    if loaded.settings.plugins.secret_key.expose().is_empty() {
        println!("warning: plugins.secret_key is not set, so no event is sent to any plugin");
    }
    Ok(())
}

pub async fn grant(
    options: &Options,
    plugin_id: &str,
    scopes: &str,
    courses: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let wanted = parse_scopes(scopes)?;
    let wanted_courses = courses.map(parse_courses).transpose()?;
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before = plugins::get(&mut tx, plugin_id).await?;
    if let Some(extra) = wanted.iter().find(|s| !before.requested_scopes.contains(s)) {
        bail!(
            "`{}` cannot be granted: {plugin_id} did not ask for it (it asked for: {})",
            extra.as_str(),
            scope_list(&before.requested_scopes)
        );
    }
    let mut sorted = wanted.clone();
    sorted.sort();
    sorted.dedup();
    let mut current = before.scopes.clone();
    current.sort();
    let courses_after = wanted_courses.unwrap_or_else(|| before.courses.clone());
    if sorted == current && courses_after == before.courses {
        println!(
            "{plugin_id} already has the scopes: {} and {}; nothing was changed",
            scope_list(&before.scopes),
            courses_text(&before.courses)
        );
        return Ok(());
    }
    if dry_run {
        println!(
            "would change {plugin_id} from scopes [{}], {} to scopes [{}], {}",
            scope_list(&before.scopes),
            courses_text(&before.courses),
            scope_list(&sorted),
            courses_text(&courses_after)
        );
        return Ok(());
    }
    let after = plugins::grant(&mut tx, plugin_id, &wanted, &courses_after).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.grant",
        "plugin",
        after.id,
        json!({
            "plugin_id": after.plugin_id,
            "from": scope_names(&before.scopes),
            "to": scope_names(&after.scopes),
            "courses_from": courses_json(&before.courses),
            "courses_to": courses_json(&after.courses),
        }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!(
        "{plugin_id} now has the scopes: {}; courses: {}",
        scope_list(&after.scopes),
        courses_text(&after.courses)
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Upgrade, token, calls (ADR-0079)
// ---------------------------------------------------------------------------------------------

fn list_line(label: &str, items: &[String]) {
    if !items.is_empty() {
        println!("  {label}: {}", items.join(", "));
    }
}

fn print_plan(plan: &plugins::UpgradePlan) {
    println!("  version: {} -> {}", plan.from_version, plan.to_version);
    list_line(
        "asks for new scopes",
        &plan
            .scopes_new
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect::<Vec<_>>(),
    );
    list_line(
        "no longer asks for",
        &plan
            .scopes_dropped
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect::<Vec<_>>(),
    );
    list_line("subscribes to now", &plan.subscribes_added);
    list_line("no longer subscribes to", &plan.subscribes_removed);
    list_line("new settings", &plan.settings_added);
    list_line("settings changed", &plan.settings_changed);
    list_line("settings gone", &plan.settings_removed);
    list_line("new routes", &plan.routes_added);
    list_line("routes changed", &plan.routes_changed);
    list_line("routes gone", &plan.routes_removed);
    if plan.pages_changed {
        println!("  its pages in the web app changed");
    }
}

fn scope_strings(scopes: &[Scope]) -> Vec<&'static str> {
    scopes.iter().map(|s| s.as_str()).collect()
}

/// Reads the manifest of an installed plugin again and takes what changed. Only the scopes it did
/// not ask for before are asked about; `--grant` answers for all of them.
pub async fn upgrade(
    options: &Options,
    plugin_id: &str,
    grant: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let installed = {
        let mut conn = pool.acquire().await.context("cannot reach the database")?;
        plugins::get(&mut conn, plugin_id).await?
    };
    // The plugin answers from where it is installed; no transaction is open while it does.
    let manifest = plugins::fetch_manifest(&installed.base_url).await?;
    if manifest.id != installed.plugin_id {
        bail!(
            "the plugin at {} is `{}`, not `{plugin_id}`: nothing was changed",
            installed.base_url,
            manifest.id
        );
    }
    let plan = plugins::plan_upgrade(&installed, &manifest);
    if plan.is_empty() {
        println!(
            "{plugin_id} {} says what it said: nothing was changed",
            plan.to_version
        );
        return Ok(());
    }
    println!("{plugin_id} changed:");
    print_plan(&plan);
    let approved = match grant {
        Some(text) => parse_scopes(text)?,
        None if dry_run => Vec::new(),
        None => ask_scopes(&plan.scopes_new)?,
    };
    if let Some(extra) = approved.iter().find(|s| !plan.scopes_new.contains(s)) {
        bail!(
            "`{}` cannot be granted here: {plugin_id} did not newly ask for it (the new ones are: {})",
            extra.as_str(),
            scope_list(&plan.scopes_new)
        );
    }
    if dry_run {
        println!("(dry run: nothing was changed)");
        return Ok(());
    }
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let done = plugins::upgrade(&mut tx, plugin_id, &manifest, &approved).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.upgrade",
        "plugin",
        done.after.id,
        json!({
            "plugin_id": done.after.plugin_id,
            "from": done.plan.from_version,
            "to": done.plan.to_version,
            "scopes_new": scope_strings(&done.plan.scopes_new),
            "scopes_granted": scope_strings(&approved),
            "scopes_dropped": scope_strings(&done.plan.scopes_dropped),
            "settings_values_dropped": done.dropped.iter().map(|d| json!({"key": d.key, "course_id": d.course})).collect::<Vec<_>>(),
        }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!(
        "{plugin_id} is now {}; scopes: {}",
        done.after.version,
        scope_list(&done.after.scopes)
    );
    let held_back: Vec<_> = plan
        .scopes_new
        .iter()
        .filter(|s| !approved.contains(s))
        .map(|s| s.as_str())
        .collect();
    if !held_back.is_empty() {
        println!("not granted (it asks for them): {}", held_back.join(", "));
    }
    for value in &done.dropped {
        println!(
            "removed the value of `{}`{}: {}",
            value.key,
            value
                .course
                .map_or(String::new(), |c| format!(" for course {c}")),
            value.why
        );
    }
    Ok(())
}

/// Gives the plugin a new token; the old one stops working at once.
pub async fn rotate_token(options: &Options, plugin_id: &str, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before = plugins::get(&mut tx, plugin_id).await?;
    if dry_run {
        println!("would give {plugin_id} a new token; its current one would stop working at once");
        return Ok(());
    }
    let token = plugins::rotate_token(&mut tx, plugin_id).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.rotate_token",
        "plugin",
        before.id,
        json!({"plugin_id": before.plugin_id}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("{plugin_id} has a new token; the old one no longer works.");
    println!("It is shown once. Give it to the plugin, for example as an environment variable:");
    println!("  VUMLO_PLUGIN_TOKEN={}", token.expose());
    Ok(())
}

/// What the plugin has called on the plugin API.
pub async fn calls(
    options: &Options,
    plugin_id: &str,
    hours: u32,
    limit: u32,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let since = chrono::Utc::now() - chrono::TimeDelta::hours(i64::from(hours));
    let report = plugins::calls(&mut conn, plugin_id, since, i64::from(limit)).await?;
    if report.counts.is_empty() {
        println!("{plugin_id} made no call in the last {hours} hour(s)");
        return Ok(());
    }
    println!("{plugin_id}, the last {hours} hour(s):");
    for count in &report.counts {
        println!(
            "  {:>6} x {:<6} {} -> {} (about {} ms)",
            count.calls, count.method, count.route, count.status, count.average_ms
        );
    }
    println!("newest first:");
    for call in &report.recent {
        println!(
            "  {} {:<6} {} -> {} ({} ms)",
            call.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            call.method,
            call.route,
            call.status,
            call.duration_ms
        );
    }
    Ok(())
}

/// `disable` and `enable`.
pub async fn set_status(
    options: &Options,
    plugin_id: &str,
    status: InstallStatus,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before: Install = plugins::get(&mut tx, plugin_id).await?;
    if before.status == status {
        println!(
            "{plugin_id} is {} already; nothing was changed",
            status.as_str()
        );
        return Ok(());
    }
    if dry_run {
        println!("would make {plugin_id} {}", status.as_str());
        return Ok(());
    }
    let after = plugins::set_status(&mut tx, plugin_id, status).await?;
    let action = match status {
        InstallStatus::Disabled => "plugin.disable",
        InstallStatus::Enabled => "plugin.enable",
    };
    db::insert_cli_audit(
        &mut tx,
        action,
        "plugin",
        after.id,
        json!({"plugin_id": after.plugin_id}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    match status {
        InstallStatus::Disabled => {
            println!("{plugin_id} is disabled: its token stops working and it is sent no event")
        }
        InstallStatus::Enabled => println!("{plugin_id} is enabled"),
    }
    Ok(())
}

pub async fn remove(
    options: &Options,
    plugin_id: &str,
    yes: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before = plugins::get(&mut tx, plugin_id).await?;
    if dry_run {
        println!("would remove {plugin_id}, with the events it is owed and the reports it wrote");
        return Ok(());
    }
    if !yes {
        bail!(
            "removing {plugin_id} deletes the events it is owed and every report it wrote; \
             say --yes to do it (or --dry-run to look)"
        );
    }
    let removed = plugins::remove(&mut tx, plugin_id).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.remove",
        "plugin",
        before.id,
        json!({"plugin_id": removed.plugin_id}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("removed {plugin_id}");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Settings (ADR-0072)
// ---------------------------------------------------------------------------------------------

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `value`, `value (default)`, `(set)` for a secret, or `(not set)`.
fn state(view: &SettingView) -> String {
    match (&view.value, view.spec.secret, view.is_set) {
        (_, true, true) => "(set)".to_owned(),
        (Some(value), _, true) => shown(value),
        (Some(value), _, false) => format!("{} (default)", shown(value)),
        (None, _, _) => "(not set)".to_owned(),
    }
}

fn kind_of(view: &SettingView) -> String {
    let spec = &view.spec;
    let base = match (spec.options.as_ref(), spec.kind) {
        (Some(options), _) => format!("one of {}", options.join("|")),
        (None, plugins::SettingKind::Array) => "list".to_owned(),
        (None, kind) => serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
    };
    if spec.secret {
        format!("{base}, secret")
    } else {
        base
    }
}

pub async fn settings_list(
    options: &Options,
    plugin_id: &str,
    course: Option<Uuid>,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let install = plugins::get(&mut conn, plugin_id).await?;
    let views = plugins::list_settings(&mut conn, plugin_id, course).await?;
    if install.settings.is_empty() {
        println!("{plugin_id} declares no settings");
    }
    for view in &views {
        println!("{:<20} {:<22} {}", view.key, kind_of(view), state(view));
        if !view.spec.description.is_empty() {
            println!("    {}", view.spec.description);
        }
    }
    // The ones of the other scope: said, not shown.
    let other = if course.is_some() {
        SettingScope::Institution
    } else {
        SettingScope::Course
    };
    for key in install
        .settings
        .iter()
        .filter(|(_, spec)| spec.scope == other)
        .map(|(key, _)| key)
    {
        println!(
            "{key:<20} per {}: {}",
            other.as_str(),
            if course.is_some() {
                "list without --course"
            } else {
                "add --course <id>"
            }
        );
    }
    Ok(())
}

pub async fn settings_get(
    options: &Options,
    plugin_id: &str,
    key: &str,
    course: Option<Uuid>,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let view = plugins::get_setting(&mut conn, plugin_id, key, course).await?;
    println!("{}", state(&view));
    Ok(())
}

/// The value as typed, or the first line of standard input when it is `-`.
fn typed(value: &str) -> anyhow::Result<String> {
    if value != "-" {
        return Ok(value.to_owned());
    }
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("cannot read the value")?;
    Ok(line.trim_end_matches(['\n', '\r']).to_owned())
}

fn audit_meta(install: &Install, key: &str, course: Option<Uuid>) -> Value {
    // Never the value: it may be a credential.
    json!({"plugin_id": install.plugin_id, "key": key, "course_id": course})
}

pub async fn settings_set(
    options: &Options,
    plugin_id: &str,
    key: &str,
    value: &str,
    course: Option<Uuid>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let secret_key = self::key(&loaded.settings)?;
    let text = typed(value)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let install = plugins::get(&mut tx, plugin_id).await?;
    // Parsed, checked, stored and announced inside the transaction; a dry run rolls it back.
    let changed =
        plugins::set_setting_text(&mut tx, &secret_key, plugin_id, key, &text, course).await?;
    if !changed {
        println!("{plugin_id} `{key}` has this value already; nothing was changed");
        return Ok(());
    }
    if dry_run {
        println!("would set `{key}` of {plugin_id}");
        return Ok(());
    }
    db::insert_cli_audit(
        &mut tx,
        "plugin.setting.set",
        "plugin",
        install.id,
        audit_meta(&install, key, course),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("set `{key}` of {plugin_id}");
    Ok(())
}

pub async fn settings_unset(
    options: &Options,
    plugin_id: &str,
    key: &str,
    course: Option<Uuid>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let install = plugins::get(&mut tx, plugin_id).await?;
    if !plugins::unset_setting(&mut tx, plugin_id, key, course).await? {
        println!("{plugin_id} `{key}` has no value; nothing was changed");
        return Ok(());
    }
    if dry_run {
        println!("would unset `{key}` of {plugin_id}");
        return Ok(());
    }
    db::insert_cli_audit(
        &mut tx,
        "plugin.setting.unset",
        "plugin",
        install.id,
        audit_meta(&install, key, course),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("unset `{key}` of {plugin_id}; the default applies");
    Ok(())
}

/// One answer line, without its line ending; `None` at the end of the input. A secret is read
/// without an echo when a person types it, and as a plain line when the input is piped.
fn read_answer(secret: bool) -> anyhow::Result<Option<String>> {
    if secret && std::io::stdin().is_terminal() {
        let text = rpassword::prompt_password("> ").context("cannot read the answer")?;
        return Ok(Some(text));
    }
    print!("> ");
    std::io::stdout()
        .flush()
        .context("cannot write the prompt")?;
    let mut line = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("cannot read the answer")?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\n', '\r']).to_owned()))
}

/// Walks through the settings of the plugin that belong to the institution, one question each: the
/// setting, what it is for and its value now. An empty answer keeps the value; one the setting
/// refuses is explained and asked again. Everything is stored in one transaction at the end, with
/// one audit row per changed setting, so input that ends midway changes nothing.
pub async fn configure(options: &Options, plugin_id: &str, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let secret_key = key(&loaded.settings)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let install = plugins::get(&mut tx, plugin_id).await?;
    let views = plugins::list_settings(&mut tx, plugin_id, None).await?;
    let per_course = install
        .settings
        .values()
        .filter(|spec| spec.scope == SettingScope::Course)
        .count();
    if per_course > 0 {
        let (count, verb) = if per_course == 1 {
            ("1 setting", "is")
        } else {
            (&*format!("{per_course} settings"), "are")
        };
        println!(
            "{count} {verb} set per course (`vumlo-cli plugin settings list {plugin_id} --course <id>`)"
        );
    }
    if views.is_empty() {
        println!("{plugin_id} declares no institution settings");
        return Ok(());
    }
    println!("Settings of {plugin_id}. Enter keeps the current value.");
    let mut changed = Vec::new();
    for view in &views {
        println!("\n{} ({})", view.key, kind_of(view));
        if !view.spec.description.is_empty() {
            println!("    {}", view.spec.description);
        }
        println!("    now: {}", state(view));
        loop {
            let Some(answer) = read_answer(view.spec.secret)? else {
                bail!("the input ended before every setting was answered; nothing was changed");
            };
            if answer.is_empty() {
                break;
            }
            // A refused answer is explained before anything is written, so the transaction is whole.
            match plugins::set_setting_text(
                &mut tx,
                &secret_key,
                plugin_id,
                &view.key,
                &answer,
                None,
            )
            .await
            {
                Ok(true) => {
                    changed.push(view.key.clone());
                    break;
                }
                Ok(false) => break,
                Err(error @ plugins::PluginsError::BadSetting { .. }) => println!("{error}"),
                Err(other) => return Err(other.into()),
            }
        }
    }
    println!();
    if changed.is_empty() {
        println!("nothing was changed");
        return Ok(());
    }
    if dry_run {
        for key in &changed {
            println!("would set `{key}` of {plugin_id}");
        }
        return Ok(());
    }
    for key in &changed {
        db::insert_cli_audit(
            &mut tx,
            "plugin.setting.set",
            "plugin",
            install.id,
            audit_meta(&install, key, None),
        )
        .await?;
    }
    tx.commit().await.context("cannot commit")?;
    for key in &changed {
        println!("set `{key}` of {plugin_id}");
    }
    Ok(())
}
