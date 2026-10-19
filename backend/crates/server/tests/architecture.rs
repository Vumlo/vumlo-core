//! Rules of ADR-0006 and ADR-0033, checked by reading the source. A module outside the owner of
//! a table never reads it: `teaching` hands out who takes part in a course through the view
//! `teaching.effective_members`, and `org` hands out its group memberships to `teaching`'s SQL
//! only. When a reader skips the view it silently misses the students that come from a group,
//! or the exclusions that take them away.
//!
//! Two more rules (AGENTS §4, ADR-0001): a module writes only its own schema, and a view reads
//! another schema's tables only where an ADR declared that interface (`VIEW_READS`). Other
//! modules' tables are otherwise not covered for reading: the gradebook reads `content` and
//! `identity` with read-only JOINs on purpose (ADR-0015, ADR-0028).

use std::fs;
use std::path::{Path, PathBuf};

/// (text no module but the owner may mention, the module that owns it)
const OWNED: &[(&str, &str)] = &[
    ("teaching.memberships", "teaching"),
    ("teaching.course_groups", "teaching"),
    ("teaching.course_exclusions", "teaching"),
    ("teaching.group_roster", "teaching"),
    ("org.group_members", "org"),
    ("org.subgroup_members", "org"),
];

/// The schemas of the modules that own tables.
const SCHEMAS: &[&str] = &[
    "identity",
    "org",
    "teaching",
    "content",
    "assessment",
    "gradebook",
    "files",
    "events",
    "idempotency",
];

/// (schema of the view, table of another schema it may read). Each is an interface an ADR
/// declared; anything else a view reads from another schema fails the test.
const VIEW_READS: &[(&str, &str)] = &[
    ("teaching", "org.group_members"),
    ("teaching", "org.subgroup_members"),
    ("teaching", "identity.users"),
];

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
    {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// Every line, as `path:line: text`, that mentions `forbidden` outside the `owner` module.
fn offences(src: &Path, forbidden: &str, owner: &str) -> Vec<String> {
    let owner = src.join(owner);
    let mut files = Vec::new();
    rust_files(src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files.into_iter().filter(|file| !file.starts_with(&owner)) {
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate() {
            if line.contains(forbidden) {
                let shown = file.strip_prefix(src).unwrap_or(&file).display();
                found.push(format!("{shown}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn no_module_reads_a_table_another_module_owns() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (_, owner) in OWNED {
        assert!(
            src.join(owner).is_dir(),
            "the source tree moved: {}",
            src.join(owner).display()
        );
    }
    for (forbidden, owner) in OWNED {
        let found = offences(&src, forbidden, owner);
        assert!(
            found.is_empty(),
            "{forbidden} is read outside the {owner} module; teaching.effective_members is the \
             roster (ARCHITECTURE.md, ADR-0033):\n{}",
            found.join("\n")
        );
    }
}

/// The module a source file belongs to: the folder under `src`. A file at the top (`main.rs`,
/// `app.rs`) belongs to none.
fn module_of(src: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(src).ok()?;
    let mut parts = relative.components();
    let first = parts.next()?.as_os_str().to_str()?.to_owned();
    parts.next().map(|_| first)
}

#[test]
fn a_module_writes_only_its_own_schema() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let owner = module_of(&src, &file);
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        // SQL spans lines; one space between words makes `INSERT INTO\n   identity.users` one phrase.
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for schema in SCHEMAS
            .iter()
            .filter(|schema| Some(**schema) != owner.as_deref())
        {
            for verb in ["INSERT INTO", "UPDATE", "DELETE FROM", "TRUNCATE"] {
                let phrase = format!("{verb} {schema}.");
                if flat.contains(&phrase) {
                    let shown = file.strip_prefix(&src).unwrap_or(&file).display();
                    found.push(format!("{shown}: `{phrase}…`"));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a module writes another module's schema; call that module's public function instead \
         (AGENTS §4):\n{}",
        found.join("\n")
    );
}

/// `schema.table` words in `sql`, in order.
fn qualified_names(sql: &str) -> Vec<String> {
    let mut names = Vec::new();
    for word in sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
        let Some((schema, table)) = word.split_once('.') else {
            continue;
        };
        if SCHEMAS.contains(&schema) && !table.is_empty() && !table.contains('.') {
            names.push(word.to_owned());
        }
    }
    names
}

#[test]
fn a_view_reads_another_schema_only_where_an_adr_declared_it() {
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
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
