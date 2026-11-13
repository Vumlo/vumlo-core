use uuid::Uuid;

use super::*;
use crate::layout::{self, Entry, GroupDef, Settings};
use crate::model::{Cell, ColumnStatus, RowTotals};

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal")
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn column(n: u128, title: &str, max: &str, group: Option<Uuid>) -> Column {
    Column {
        assignment_id: id(n),
        title: title.to_owned(),
        max_points: d(max),
        is_bonus: false,
        status: ColumnStatus::Published,
        group,
    }
}

fn cell(draft: Option<&str>, published: Option<&str>) -> Cell {
    Cell {
        draft_score: draft.map(d),
        draft_feedback: None,
        published_score: published.map(d),
        published_feedback: None,
        has_unpublished_changes: draft != published,
    }
}

/// A student of a course with two assignments in one group, 20 points each, the target 30 and the
/// pass mark 15. The first has a published 18, the second only a draft 20.
fn fixture(name: &str, former: bool) -> (Vec<Column>, LayoutView, Vec<GradebookRow>) {
    let group = GroupDef::plain(id(100), "Labs");
    let settings = Settings {
        target: d("30"),
        pass_mark: Some(d("15")),
        ..Settings::default()
    };
    let columns = vec![
        column(1, "Lab 1", "20", Some(id(100))),
        column(2, "Lab 2", "20", Some(id(100))),
    ];
    let entry = |n: u128, score: Option<&str>| Entry {
        assignment_id: id(n),
        max_points: d("20"),
        is_bonus: false,
        group: Some(id(100)),
        score: score.map(d),
    };
    let published = layout::compute(
        &settings,
        std::slice::from_ref(&group),
        &[entry(1, Some("18")), entry(2, None)],
    );
    let draft = layout::compute(
        &settings,
        std::slice::from_ref(&group),
        &[entry(1, Some("18")), entry(2, Some("20"))],
    );
    let view = LayoutView {
        setup: layout::setup(&settings, std::slice::from_ref(&group), &[]),
        possible: d("40"),
        groups: vec![group],
        settings,
    };
    let rows = vec![GradebookRow {
        user_id: id(9),
        name: name.to_owned(),
        surname: "Kovalenko".to_owned(),
        former,
        cells: vec![
            (id(1), cell(Some("18"), Some("18"))),
            (id(2), cell(Some("20"), None)),
        ],
        totals: Some(RowTotals { published, draft }),
    }];
    (columns, view, rows)
}

#[test]
fn only_published_marks_and_the_published_total_are_in_the_table() {
    let (columns, view, rows) = fixture("Olena", false);
    let t = table(&columns, Some(&view), &rows);

    assert_eq!(
        t.header,
        [
            "Surname",
            "Name",
            "Lab 1 (max 20)",
            "Lab 2 (max 20)",
            "Labs",
            "Total",
            "Earned",
            "Passed",
            "Note"
        ]
    );
    assert_eq!(
        t.rows[0],
        [
            Value::Text("Kovalenko".into()),
            Value::Text("Olena".into()),
            Value::Number(d("18")),
            Value::Blank,
            Value::Number(d("18")),
            Value::Number(d("18")),
            Value::Number(d("18")),
            Value::Text("yes".into()),
            Value::Blank,
        ],
        "the draft 20 is not in the file"
    );
}

#[test]
fn the_total_is_cut_at_the_target_and_the_points_earned_are_kept() {
    let (columns, view, mut rows) = fixture("Olena", false);
    let group = view.groups[0].clone();
    let entry = |n: u128, score: &str| Entry {
        assignment_id: id(n),
        max_points: d("20"),
        is_bonus: false,
        group: Some(id(100)),
        score: Some(d(score)),
    };
    let full = layout::compute(
        &view.settings,
        std::slice::from_ref(&group),
        &[entry(1, "20"), entry(2, "20")],
    );
    rows[0].totals = Some(RowTotals {
        published: full.clone(),
        draft: full,
    });
    let t = table(&columns, Some(&view), &rows);
    // Total, Earned: 30 of a target of 30, and the 40 earned.
    assert_eq!(t.rows[0][5], Value::Number(d("30")));
    assert_eq!(t.rows[0][6], Value::Number(d("40")));
}

#[test]
fn a_student_who_left_is_marked_and_a_name_that_looks_like_a_formula_stays_text() {
    let (columns, view, rows) = fixture("=HYPERLINK(\"http://x\")", true);
    let t = table(&columns, Some(&view), &rows);
    assert_eq!(t.rows[0][1], Value::Text("=HYPERLINK(\"http://x\")".into()));
    assert_eq!(
        t.rows[0].last(),
        Some(&Value::Text("left the course".into()))
    );
    let bytes = workbook(&t).expect("a workbook");
    assert_eq!(&bytes[..2], b"PK", "an xlsx file is a zip archive");
}

#[test]
fn the_twelve_point_scale_has_no_totals() {
    let (columns, _, rows) = fixture("Olena", false);
    let t = table(&columns, None, &rows);
    assert_eq!(t.header.last().map(String::as_str), Some("Note"));
    assert_eq!(t.header.len(), 5);
    assert_eq!(t.rows[0].len(), 5);
}
