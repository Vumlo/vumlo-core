//! The layer rules that Cargo cannot see (ADR-0065). Cargo decides who may depend on whom; this
//! file decides what code in a layer may do with the database and with a few functions that are
//! dangerous in the wrong hands. It reads the source, so it needs no database.
//!
//! - A unit writes only its own schema.
//! - A unit reads the tables of its own layer, and those of a lower layer only through the views
//!   that layer declared (`DECLARED_VIEWS`). It never names the schema of a higher layer.
//! - The tables that hold who is on a course (`OWNED`) are read only by their owner: the roster is
//!   `teaching.effective_members`, and a reader that skips it misses the students that come from a
//!   group, or the exclusions that take them away (ADR-0033).
//! - A view in a migration reads another schema only where an ADR declared that interface.
//! - `Actor::role` is read only by `access`; `auth::apply_user_update` is called only by `learning`.
//! - The `testing` feature of a crate is switched on only from `[dev-dependencies]`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A part of the backend that owns code, and maybe a schema. When the code moves into a crate of
/// its own, only `dir` changes.
struct Unit {
    name: &'static str,
    /// Relative to `backend/crates`. The longest `dir` that contains a file decides its unit.
    dir: &'static str,
    schema: Option<&'static str>,
    /// Higher layers may use lower ones, never the other way round. The four `learning` modules
    /// share a layer: they may read one another (ADR-0065).
    layer: u8,
}

const UNITS: &[Unit] = &[
    Unit {
        name: "common",
        dir: "common/src",
        schema: None,
        layer: 0,
    },
    Unit {
        name: "access",
        dir: "access/src",
        schema: None,
        layer: 1,
    },
    Unit {
        name: "events",
        dir: "events/src",
        schema: Some("events"),
        layer: 2,
    },
    Unit {
        name: "auth",
        dir: "auth/src",
        schema: Some("auth"),
        layer: 3,
    },
    Unit {
        name: "org",
        dir: "org/src",
        schema: Some("org"),
        layer: 4,
    },
    Unit {
        name: "teaching",
        dir: "server/src/learning/teaching",
        schema: Some("teaching"),
        layer: 5,
    },
    Unit {
        name: "content",
        dir: "server/src/learning/content",
        schema: Some("content"),
        layer: 5,
    },
    Unit {
        name: "assessment",
        dir: "server/src/learning/assessment",
        schema: Some("assessment"),
        layer: 5,
    },
    Unit {
        name: "gradebook",
        dir: "server/src/learning/gradebook",
        schema: Some("gradebook"),
        layer: 5,
    },
    // The flows above those four (a person with their courses, the audit report, ...): no schema.
    Unit {
        name: "learning",
        dir: "server/src/learning",
        schema: None,
        layer: 5,
    },
    // The composition root and the middleware that has a table of its own.
    Unit {
        name: "server",
        dir: "server/src",
        schema: Some("idempotency"),
        layer: 6,
    },
];

/// What a layer offers the ones above it to read. Each is a view its owner declared in a
/// migration, so the owner can change the table behind it.
const DECLARED_VIEWS: &[&str] = &[
    "auth.people",
    "org.term_list",
    "org.group_list",
    "org.subgroup_list",
    "events.audit_entries",
];

/// (text no unit but the owner may mention, the unit that owns it)
const OWNED: &[(&str, &str)] = &[
    ("teaching.memberships", "teaching"),
    ("teaching.course_groups", "teaching"),
    ("teaching.course_exclusions", "teaching"),
    ("teaching.group_roster", "teaching"),
    ("org.group_members", "org"),
    ("org.subgroup_members", "org"),
];

/// Schemas that belong to no unit of the code (an empty one that migrations still create).
const OTHER_SCHEMAS: &[&str] = &["files"];

/// (schema of the view, table of another schema it may read). Each is an interface an ADR
/// declared; anything else a view reads from another schema fails the test.
const VIEW_READS: &[(&str, &str)] = &[
    ("teaching", "org.group_members"),
    ("teaching", "org.subgroup_members"),
    ("teaching", "auth.users"),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the server crate lies in backend/crates")
        .to_path_buf()
}

fn schemas() -> Vec<&'static str> {
    UNITS
        .iter()
        .filter_map(|unit| unit.schema)
        .chain(OTHER_SCHEMAS.iter().copied())
        .collect()
}

fn rust_files(dir: &Path, found: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
    {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.insert(path);
        }
    }
}

/// Every Rust file of every unit.
fn source_files() -> Vec<PathBuf> {
    let crates = crates_dir();
    let mut found = BTreeSet::new();
    for unit in UNITS {
        let dir = crates.join(unit.dir);
        assert!(dir.is_dir(), "the source tree moved: {}", dir.display());
        rust_files(&dir, &mut found);
    }
    found.into_iter().collect()
}

fn unit_of(file: &Path) -> &'static Unit {
    let relative = file
        .strip_prefix(crates_dir())
        .expect("a source file lies under backend/crates");
    UNITS
        .iter()
        .filter(|unit| relative.starts_with(unit.dir))
        .max_by_key(|unit| unit.dir.len())
        .unwrap_or_else(|| panic!("{} belongs to no unit", file.display()))
}

fn shown(file: &Path) -> String {
    file.strip_prefix(crates_dir())
        .unwrap_or(file)
        .display()
        .to_string()
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// A test or its support: not part of what ships.
fn is_test_code(file: &Path) -> bool {
    file.file_name()
        .is_some_and(|name| name == "tests.rs" || name == "test_support.rs")
}

/// A file that holds SQL: AGENTS section 4 puts all of it in `repo.rs` (or a `repo/` folder).
fn is_repo(file: &Path) -> bool {
    !is_test_code(file)
        && (file.file_name().is_some_and(|name| name == "repo.rs")
            || file.components().any(|part| part.as_os_str() == "repo"))
}

/// Every line, as `path:line: text`, that mentions `forbidden` outside the `owner` unit.
fn offences(forbidden: &str, owner: &str) -> Vec<String> {
    let mut found = Vec::new();
    for file in source_files()
        .into_iter()
        .filter(|f| unit_of(f).name != owner)
    {
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate() {
            if line.contains(forbidden) {
                found.push(format!("{}:{}: {}", shown(&file), number + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn no_module_reads_a_table_another_module_owns() {
    for (forbidden, owner) in OWNED {
        let found = offences(forbidden, owner);
        assert!(
            found.is_empty(),
            "{forbidden} is read outside {owner}; teaching.effective_members is the roster \
             (ARCHITECTURE.md, ADR-0033):\n{}",
            found.join("\n")
        );
    }
}

#[test]
fn a_module_writes_only_its_own_schema() {
    let mut found = Vec::new();
    for file in source_files() {
        let own = unit_of(&file).schema;
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        // SQL spans lines; one space between words makes `INSERT INTO\n   auth.users` one phrase.
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for schema in schemas().into_iter().filter(|schema| Some(*schema) != own) {
            for verb in ["INSERT INTO", "UPDATE", "DELETE FROM", "TRUNCATE"] {
                let phrase = format!("{verb} {schema}.");
                if flat.contains(&phrase) {
                    found.push(format!("{}: `{phrase}…`", shown(&file)));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a unit writes another unit's schema; call that unit's public function instead \
         (AGENTS section 4):\n{}",
        found.join("\n")
    );
}

/// `schema.table` words in `sql`, in order.
fn qualified_names(sql: &str) -> Vec<String> {
    let known = schemas();
    let mut names = Vec::new();
    for word in sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
        let Some((schema, table)) = word.split_once('.') else {
            continue;
        };
        if known.contains(&schema) && !table.is_empty() && !table.contains('.') {
            names.push(word.to_owned());
        }
    }
    names
}

fn layer_of_schema(schema: &str) -> Option<u8> {
    UNITS
        .iter()
        .find(|unit| unit.schema == Some(schema))
        .map(|unit| unit.layer)
}

#[test]
fn a_unit_reads_its_own_layer_and_the_declared_views_below_it() {
    let mut found = Vec::new();
    let mut checked = 0;
    for file in source_files().into_iter().filter(|f| is_repo(f)) {
        let unit = unit_of(&file);
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate().filter(|(_, l)| !is_comment(l)) {
            for name in qualified_names(line) {
                checked += 1;
                let schema = name.split_once('.').map_or("", |(schema, _)| schema);
                let allowed = match layer_of_schema(schema) {
                    None => false,
                    Some(layer) if Some(schema) == unit.schema || layer == unit.layer => true,
                    Some(layer) if layer < unit.layer => DECLARED_VIEWS.contains(&name.as_str()),
                    Some(_) => false,
                };
                if !allowed {
                    found.push(format!(
                        "{}:{}: {name} (in {})",
                        shown(&file),
                        number + 1,
                        unit.name
                    ));
                }
            }
        }
    }
    assert!(
        checked > 100,
        "the scan found only {checked} table names; it is broken"
    );
    assert!(
        found.is_empty(),
        "a unit reads a table of a lower layer that is not a declared view, or one of a higher \
         layer (ADR-0065; declare a view in a migration and list it in DECLARED_VIEWS):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_role_of_an_actor_is_read_only_by_access() {
    let mut found = Vec::new();
    for file in source_files()
        .into_iter()
        .filter(|f| unit_of(f).name != "access")
    {
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate().filter(|(_, l)| !is_comment(l)) {
            if line.contains(".role()") || line.contains("Actor::role") {
                found.push(format!("{}:{}: {}", shown(&file), number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "roles are read only by access; ask access::authorize (AGENTS section 5):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_primitive_that_changes_a_user_is_called_only_from_learning() {
    let mut found = Vec::new();
    for file in source_files() {
        if matches!(unit_of(&file).name, "auth" | "learning") {
            continue;
        }
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate().filter(|(_, l)| !is_comment(l)) {
            if line.contains("apply_user_update") {
                found.push(format!("{}:{}: {}", shown(&file), number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "auth::apply_user_update does not know the courses of the user; call \
         learning::people::update_user (ADR-0037):\n{}",
        found.join("\n")
    );
}

/// Every `Cargo.toml` of the backend.
fn manifests() -> Vec<PathBuf> {
    let crates = crates_dir();
    let root = crates.parent().expect("backend");
    let mut found = vec![root.join("Cargo.toml")];
    for dir in [crates.to_path_buf(), crates.join("modules")] {
        for entry in
            fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        {
            let manifest = entry.expect("directory entry").path().join("Cargo.toml");
            if manifest.is_file() {
                found.push(manifest);
            }
        }
    }
    found
}

#[test]
fn the_testing_feature_is_switched_on_only_from_dev_dependencies() {
    let mut found = Vec::new();
    for manifest in manifests() {
        let text = fs::read_to_string(&manifest).expect("a manifest is UTF-8");
        let mut section = String::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.starts_with('[') {
                section = line.trim_matches(|c| c == '[' || c == ']').to_owned();
                continue;
            }
            if !line.contains("\"testing\"") {
                continue;
            }
            // `[features]` may list it as a feature of its own (`testing = []`); a dependency may
            // switch it on only in a dev-dependency table.
            let in_dev = section.ends_with("dev-dependencies");
            let in_dependency = section.ends_with("dependencies");
            let in_default = section == "features" && line.starts_with("default");
            if (in_dependency && !in_dev) || in_default {
                found.push(format!("{}:{}: {line}", shown(&manifest), number + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "the testing feature opens helpers that must not ship; enable it only under \
         [dev-dependencies]:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_view_reads_another_schema_only_where_an_adr_declared_it() {
    let migrations = crates_dir().join("../migrations");
    let mut files: Vec<PathBuf> = fs::read_dir(&migrations)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", migrations.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no migrations found");
    let mut found = Vec::new();
    let mut views = 0;
    for file in files {
        let text = fs::read_to_string(&file).expect("a migration is UTF-8");
        // Comments may mention any table; the statements are what count.
        let code: String = text
            .lines()
            .map(|line| line.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        for statement in code.split(';') {
            let words: Vec<&str> = statement.split_whitespace().collect();
            let Some(at) = words.iter().position(|w| w.eq_ignore_ascii_case("VIEW")) else {
                continue;
            };
            if !words[..at].iter().any(|w| w.eq_ignore_ascii_case("CREATE")) {
                continue;
            }
            let Some(name) = words.get(at + 1) else {
                continue;
            };
            let Some((view_schema, _)) = name.split_once('.') else {
                continue;
            };
            views += 1;
            for read in qualified_names(&words[at + 2..].join(" ")) {
                let own = read
                    .split_once('.')
                    .is_some_and(|(schema, _)| schema == view_schema);
                if !own && !VIEW_READS.contains(&(view_schema, read.as_str())) {
                    let shown = file.file_name().unwrap_or_default().to_string_lossy();
                    found.push(format!("{shown}: view {name} reads {read}"));
                }
            }
        }
    }
    assert!(views > 0, "the scan found no view; the parser is broken");
    assert!(
        found.is_empty(),
        "a view reads a table of another schema that no ADR declared (add it to VIEW_READS only \
         with an ADR):\n{}",
        found.join("\n")
    );
}

#[test]
fn every_declared_view_exists_in_a_migration() {
    let migrations = crates_dir().join("../migrations");
    let mut all = String::new();
    for entry in fs::read_dir(&migrations).expect("the migrations folder") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_some_and(|ext| ext == "sql") {
            all.push_str(&fs::read_to_string(path).expect("a migration is UTF-8"));
            all.push('\n');
        }
    }
    let flat = all.split_whitespace().collect::<Vec<_>>().join(" ");
    for view in DECLARED_VIEWS {
        assert!(
            flat.contains(&format!("CREATE VIEW {view} ")),
            "{view} is declared here but no migration creates it"
        );
    }
}
