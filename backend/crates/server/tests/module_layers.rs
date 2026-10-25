//! The crate rules of modules and plugins (ADR-0066), checked on `cargo metadata`. Cargo already
//! refuses a cycle of crates; this decides who may depend on whom beside the chain of ADR-0065.
//!
//! - No crate of the core depends on a module. Only `server` and `vumlo-cli` do, and only through an
//!   optional dependency (the Cargo feature of the module).
//! - Only `server`, `vumlo-cli` and `plugins` depend on `module-api`; `module-api` depends on
//!   `common`, `access`, `events` and `vumlo-config` and nothing else.
//! - A module depends on `module-api` and on the layers below `learning`, never on `server`,
//!   `vumlo-cli` or `plugins`.
//! - A module depends on another module only if it lists it in `requires` (checked where the
//!   module is compiled in, so in the CI row that builds it), and sees only what that module
//!   re-exports: its `lib.rs` has no `pub mod`.
//! - A module's migrations name only its own schema `mod_<id>`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Dep {
    name: String,
    /// `None` for a normal dependency, `"dev"` or `"build"`.
    kind: Option<String>,
    optional: bool,
}

struct Package {
    name: String,
    module: bool,
    deps: Vec<Dep>,
}

/// Packages that may depend on a module crate, each only through an optional dependency.
const COMPOSITION_ROOTS: &[&str] = &["server", "vumlo-cli"];
/// What `module-api` may depend on among the crates of this workspace.
const MODULE_API_DEPS: &[&str] = &["common", "access", "events", "vumlo-config"];
/// Crates a module must not depend on, other than in its tests.
const MODULE_FORBIDDEN: &[&str] = &["server", "vumlo-cli", "plugins"];
const MODULE_API_USERS: &[&str] = &["server", "vumlo-cli", "plugins"];

fn backend() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the server crate lies in backend/crates")
        .to_path_buf()
}

/// The id of the module in crate `vumlo-module-<id>`.
fn module_id(package: &str) -> String {
    package
        .strip_prefix("vumlo-module-")
        .unwrap_or(package)
        .replace('-', "_")
}

fn packages() -> Vec<Package> {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(backend())
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("cargo metadata is JSON");
    parse(&metadata)
}

fn parse(metadata: &Value) -> Vec<Package> {
    let members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let all: Vec<&Value> = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|p| members.contains(p["id"].as_str().unwrap_or_default()))
        .collect();
    let names: BTreeSet<&str> = all.iter().filter_map(|p| p["name"].as_str()).collect();
    all.iter()
        .map(|package| Package {
            name: package["name"].as_str().expect("name").to_owned(),
            module: package["manifest_path"]
                .as_str()
                .is_some_and(|path| path.contains("/crates/modules/")),
            deps: package["dependencies"]
                .as_array()
                .expect("dependencies")
                .iter()
                .filter(|dep| names.contains(dep["name"].as_str().unwrap_or_default()))
                .map(|dep| Dep {
                    name: dep["name"].as_str().expect("name").to_owned(),
                    kind: dep["kind"].as_str().map(str::to_owned),
                    optional: dep["optional"].as_bool().unwrap_or(false),
                })
                .collect(),
        })
        .collect()
}

/// Every broken rule, one line each. `requires(id)` is what the module with that id declares, or
/// `None` where the module is not compiled in and its manifest cannot be read.
fn violations(packages: &[Package], requires: &dyn Fn(&str) -> Option<Vec<String>>) -> Vec<String> {
    let is_module = |name: &str| packages.iter().any(|p| p.module && p.name == name);
    let mut found = Vec::new();
    for package in packages {
        let ships = |dep: &&Dep| dep.kind.as_deref() != Some("dev");
        for dep in package.deps.iter().filter(|dep| is_module(&dep.name)) {
            if package.module {
                if ships(&dep) {
                    match requires(&module_id(&package.name)) {
                        Some(list) if list.contains(&module_id(&dep.name)) => {}
                        Some(_) => found.push(format!(
                            "{} depends on the module {}, which its manifest does not list in `requires`",
                            package.name, dep.name
                        )),
                        None => {}
                    }
                }
            } else if !COMPOSITION_ROOTS.contains(&package.name.as_str()) {
                found.push(format!(
                    "{} is part of the core and depends on the module {}",
                    package.name, dep.name
                ));
            } else if dep.kind.is_some() || !dep.optional {
                found.push(format!(
                    "{} must depend on the module {} only through an optional dependency \
                     (its Cargo feature)",
                    package.name, dep.name
                ));
            }
        }
        for dep in package.deps.iter().filter(|dep| dep.name == "module-api") {
            let allowed = MODULE_API_USERS.contains(&package.name.as_str()) || package.module;
            if !allowed && dep.kind.is_none() {
                found.push(format!("{} depends on module-api", package.name));
            }
        }
        if package.name == "module-api" {
            for dep in package.deps.iter().filter(ships) {
                if !MODULE_API_DEPS.contains(&dep.name.as_str()) {
                    found.push(format!(
                        "module-api depends on {}; it may depend on {} only",
                        dep.name,
                        MODULE_API_DEPS.join(", ")
                    ));
                }
            }
        }
        if package.module {
            for dep in package.deps.iter().filter(ships) {
                if MODULE_FORBIDDEN.contains(&dep.name.as_str()) {
                    found.push(format!(
                        "the module {} depends on {}",
                        package.name, dep.name
                    ));
                }
            }
        }
    }
    found
}

#[cfg(feature = "dummy")]
fn declared_requires(id: &str) -> Option<Vec<String>> {
    server::modules::compiled()
        .iter()
        .find(|m| m.manifest().id == id)
        .map(|m| {
            m.manifest()
                .requires
                .iter()
                .map(|r| (*r).to_owned())
                .collect()
        })
}

#[cfg(not(feature = "dummy"))]
fn declared_requires(_: &str) -> Option<Vec<String>> {
    None
}

#[test]
fn the_crates_depend_on_each_other_as_adr_0066_says() {
    let found = violations(&packages(), &declared_requires);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_tree_has_the_crates_the_rules_are_about() {
    let packages = packages();
    for name in ["module-api", "server", "vumlo-cli", "vumlo-module-dummy"] {
        assert!(packages.iter().any(|p| p.name == name), "no crate {name}");
    }
    let dummy = packages
        .iter()
        .find(|p| p.name == "vumlo-module-dummy")
        .unwrap();
    assert!(dummy.module, "the dummy module lies under crates/modules");
}

// --- the rules themselves, on metadata made to break them -----------------------------------------

fn package(name: &str, module: bool, deps: &[(&str, Option<&str>, bool)]) -> Package {
    Package {
        name: name.to_owned(),
        module,
        deps: deps
            .iter()
            .map(|(name, kind, optional)| Dep {
                name: (*name).to_owned(),
                kind: kind.map(str::to_owned),
                optional: *optional,
            })
            .collect(),
    }
}

fn no_manifest(_: &str) -> Option<Vec<String>> {
    None
}

#[test]
fn a_well_formed_tree_has_no_violation() {
    let tree = [
        package("learning", false, &[("common", None, false)]),
        package(
            "module-api",
            false,
            &[("common", None, false), ("events", None, false)],
        ),
        package(
            "vumlo-module-a",
            true,
            &[("module-api", None, false), ("learning", None, false)],
        ),
        package(
            "vumlo-module-b",
            true,
            &[("module-api", None, false), ("vumlo-module-a", None, false)],
        ),
        package(
            "server",
            false,
            &[
                ("module-api", None, false),
                ("vumlo-module-a", None, true),
                ("vumlo-module-b", None, true),
            ],
        ),
    ];
    let requires = |id: &str| (id == "b").then(|| vec!["a".to_owned()]);
    assert_eq!(violations(&tree, &requires), Vec::<String>::new());
}

#[test]
fn a_core_crate_that_depends_on_a_module_is_found() {
    let tree = [
        package("vumlo-module-a", true, &[]),
        package("learning", false, &[("vumlo-module-a", None, false)]),
        package("auth", false, &[("vumlo-module-a", Some("dev"), false)]),
    ];
    let found = violations(&tree, &no_manifest);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|line| line.contains("part of the core")));
}

#[test]
fn the_composition_roots_may_name_a_module_only_through_its_feature() {
    let tree = [
        package("vumlo-module-a", true, &[]),
        package("server", false, &[("vumlo-module-a", None, false)]),
        package("vumlo-cli", false, &[("vumlo-module-a", None, true)]),
    ];
    let found = violations(&tree, &no_manifest);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("server must depend on the module"));
}

#[test]
fn module_api_depends_on_three_layers_and_the_config_and_nothing_else() {
    let tree = [package(
        "module-api",
        false,
        &[
            ("common", None, false),
            ("learning", None, false),
            ("test-support", Some("dev"), false),
        ],
    )];
    let found = violations(&tree, &no_manifest);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("module-api depends on learning"));
}

#[test]
fn a_module_may_not_see_server_the_cli_or_plugins_and_the_core_may_not_see_module_api() {
    let tree = [
        package(
            "vumlo-module-a",
            true,
            &[("server", None, false), ("plugins", Some("dev"), false)],
        ),
        package("org", false, &[("module-api", None, false)]),
        package("server", false, &[]),
        package("plugins", false, &[]),
    ];
    let found = violations(&tree, &no_manifest);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .any(|l| l.contains("the module vumlo-module-a depends on server"))
    );
    assert!(
        found
            .iter()
            .any(|l| l.contains("org depends on module-api"))
    );
}

#[test]
fn a_module_depends_on_another_only_if_it_requires_it() {
    let tree = [
        package("vumlo-module-a", true, &[]),
        package("vumlo-module-b", true, &[("vumlo-module-a", None, false)]),
    ];
    let requires_nothing = |_: &str| Some(Vec::new());
    let found = violations(&tree, &requires_nothing);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("does not list in `requires`"));
    // Where the module is not compiled in, its manifest cannot be read: the CI row that builds it
    // decides.
    assert!(violations(&tree, &no_manifest).is_empty());
}

// --- what a module shows, and what its migrations touch --------------------------------------------

fn module_dirs() -> Vec<PathBuf> {
    let dir = backend().join("crates/modules");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn crate_name(dir: &Path) -> String {
    dir.file_name()
        .expect("a directory name")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_module_shows_only_what_its_lib_re_exports() {
    let mut found = Vec::new();
    for dir in module_dirs() {
        let lib = dir.join("src/lib.rs");
        let text = fs::read_to_string(&lib).expect("a module has a lib.rs");
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("pub mod ") {
                found.push(format!("{}:{}: {}", lib.display(), number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a module's lib.rs has no `pub mod`: another module may use its re-exports only \
         (ADR-0066):\n{}",
        found.join("\n")
    );
}

/// The tables and schemas a migration's statements name that are not in `schema`: the word after
/// each keyword that takes a table. Comments are ignored. It reads words, not SQL, so it knows
/// the few places where those keywords mean something else (`ON UPDATE`, `FOR UPDATE`,
/// `DISTINCT FROM`).
fn outside_schema(sql: &str, schema: &str) -> Vec<String> {
    let text = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "(),;".contains(c))
        .filter(|word| !word.is_empty())
        .collect();
    let prefix = format!("{schema}.");
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        let before = |back: usize| at.checked_sub(back).map(|i| words[i]);
        let takes_a_table = match *word {
            "table" | "view" | "references" | "join" | "into" => true,
            "from" => before(1) != Some("distinct"),
            "update" => !matches!(before(1), Some("on" | "for")),
            // `CREATE [UNIQUE] INDEX [IF NOT EXISTS] name ON table`: the index is on a table.
            "on" => (1..=5).any(|back| before(back) == Some("index")),
            "schema" => {
                found
                    .push("schema: the migrator creates mod_<id>, a migration does not".to_owned());
                false
            }
            _ => false,
        };
        if !takes_a_table {
            continue;
        }
        let target = words[at + 1..]
            .iter()
            .find(|w| !matches!(**w, "if" | "not" | "exists" | "only" | "concurrently"));
        if let Some(target) = target
            && !target.starts_with(&prefix)
        {
            found.push(format!("{word} {target}"));
        }
    }
    found
}

#[test]
fn the_scanner_finds_a_table_outside_the_schema_of_the_module() {
    let sql = "-- about core.things\n\
               CREATE TABLE mod_x.a (id uuid REFERENCES mod_x.b (id), who uuid REFERENCES auth.users (id));\n\
               INSERT INTO teaching.courses SELECT 1;\n\
               CREATE UNIQUE INDEX IF NOT EXISTS i ON org.groups (name);\n\
               CREATE SCHEMA mod_x;";
    let found = outside_schema(sql, "mod_x");
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found.contains(&"references auth.users".to_owned()));
    assert!(found.contains(&"into teaching.courses".to_owned()));
    assert!(found.contains(&"on org.groups".to_owned()));
    assert!(found.iter().any(|f| f.starts_with("schema")));
}

#[test]
fn the_scanner_accepts_what_a_module_may_write() {
    let sql = "CREATE TABLE IF NOT EXISTS mod_x.a (\n\
                 id uuid PRIMARY KEY,\n\
                 b uuid REFERENCES mod_x.b (id) ON DELETE CASCADE ON UPDATE CASCADE,\n\
                 CHECK (name IS DISTINCT FROM 'x')\n\
               );\n\
               CREATE INDEX a_b ON mod_x.a (b);\n\
               INSERT INTO mod_x.a SELECT id FROM mod_x.b FOR UPDATE;\n\
               UPDATE mod_x.a SET b = NULL; -- ALTER TABLE auth.users";
    assert_eq!(outside_schema(sql, "mod_x"), Vec::<String>::new());
}

#[test]
fn a_module_migrates_only_its_own_schema() {
    let mut found = Vec::new();
    for dir in module_dirs() {
        let schema = format!("mod_{}", module_id(&crate_name(&dir)));
        let migrations = dir.join("migrations");
        if !migrations.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&migrations).expect("read migrations") {
            let path = entry.expect("directory entry").path();
            if path.extension().is_none_or(|ext| ext != "sql") {
                continue;
            }
            let sql = fs::read_to_string(&path).expect("a migration is UTF-8");
            for line in outside_schema(&sql, &schema) {
                found.push(format!("{}: {line}", path.display()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "a module's migrations name only mod_<id> (ADR-0066):\n{}",
        found.join("\n")
    );
}
