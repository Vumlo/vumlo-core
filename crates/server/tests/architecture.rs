//! Rules of ADR-0006 and ADR-0033, checked by reading the source. A module outside the owner of
//! a table never reads it: `teaching` hands out who takes part in a course through the view
//! `teaching.effective_members`, and `org` hands out its group memberships to `teaching`'s SQL
//! only. When a reader skips the view it silently misses the students that come from a group,
//! or the exclusions that take them away.
//!
//! These are the only rules here. Other modules' tables are not covered: the gradebook reads
//! `content` and `identity` with read-only JOINs on purpose (ADR-0015, ADR-0028).

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
