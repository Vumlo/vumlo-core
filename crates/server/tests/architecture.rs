//! One rule of ADR-0006, checked by reading the source: a module outside `teaching` reads who
//! takes part in a course through the view `teaching.effective_members`, never from the table
//! `teaching.memberships`. When groups arrive only the view changes, so a reader that skipped it
//! would silently miss the students that come from a group.
//!
//! This is the only rule here. Other modules' tables are not covered: the gradebook reads
//! `content` and `identity` with read-only JOINs on purpose (ADR-0015, ADR-0028).

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN: &str = "teaching.memberships";

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

/// Every line, as `path:line: text`, that mentions the table outside the `teaching` module.
fn offences(src: &Path) -> Vec<String> {
    let teaching = src.join("teaching");
    let mut files = Vec::new();
    rust_files(src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files
        .into_iter()
        .filter(|file| !file.starts_with(&teaching))
    {
        let text = fs::read_to_string(&file).expect("a Rust file is UTF-8");
        for (number, line) in text.lines().enumerate() {
            if line.contains(FORBIDDEN) {
                let shown = file.strip_prefix(src).unwrap_or(&file).display();
                found.push(format!("{shown}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn no_module_outside_teaching_reads_the_memberships_table() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        src.join("teaching").is_dir(),
        "the source tree moved: {}",
        src.display()
    );
    let found = offences(&src);
    assert!(
        found.is_empty(),
        "{FORBIDDEN} is read outside the teaching module; read teaching.effective_members \
         instead (ARCHITECTURE.md, teaching):\n{}",
        found.join("\n")
    );
}
