//! The gradebook as a workbook (ADR-0090). [`table`] decides what is in it, from published values
//! only; [`workbook`] draws it. Neither touches the database.

use rust_xlsxwriter::{Format, Workbook, XlsxError};
use sqlx::types::Decimal;

use super::model::{Column, GradebookRow, LayoutView};

/// One cell. A text is always written as a string, never as a formula, so a name that starts with
/// `=`, `+`, `-` or `@` stays text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Blank,
    Text(String),
    Number(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Students by the order they come in, one column per assignment, then (on the hundred scale) a
/// subtotal per group, the total, the points earned before the target cut them, whether the pass
/// mark is reached, and a note for a student who left. Only published marks are read, so a draft
/// is never in the file.
pub fn table(columns: &[Column], layout: Option<&LayoutView>, rows: &[GradebookRow]) -> Table {
    let mut header = vec!["Surname".to_owned(), "Name".to_owned()];
    for column in columns {
        let extra = if column.is_bonus { ", extra" } else { "" };
        header.push(format!(
            "{} (max {}{extra})",
            column.title, column.max_points
        ));
    }
    let loose = columns.iter().any(|column| column.group.is_none());
    if let Some(layout) = layout {
        header.extend(layout.groups.iter().map(|group| group.title.clone()));
        if loose {
            header.push("Not sorted".to_owned());
        }
        header.extend(["Total", "Earned", "Passed"].map(str::to_owned));
    }
    header.push("Note".to_owned());

    let rows = rows
        .iter()
        .map(|row| {
            let mut cells = vec![
                Value::Text(row.surname.clone()),
                Value::Text(row.name.clone()),
            ];
            for column in columns {
                let score = row
                    .cells
                    .iter()
                    .find(|(id, _)| *id == column.assignment_id)
                    .and_then(|(_, cell)| cell.published_score);
                cells.push(score.map_or(Value::Blank, Value::Number));
            }
            if let Some(layout) = layout {
                let outcome = row.totals.as_ref().map(|totals| &totals.published);
                let subtotal = |key| {
                    outcome
                        .and_then(|o| o.groups.iter().find(|g| g.key == key))
                        .map_or(Value::Blank, |g| Value::Number(g.total))
                };
                cells.extend(layout.groups.iter().map(|group| subtotal(Some(group.key))));
                if loose {
                    cells.push(subtotal(None));
                }
                cells.push(outcome.map_or(Value::Blank, |o| Value::Number(o.total)));
                cells.push(outcome.map_or(Value::Blank, |o| Value::Number(o.raw_total)));
                cells.push(match outcome.and_then(|o| o.passed) {
                    Some(true) => Value::Text("yes".to_owned()),
                    Some(false) => Value::Text("no".to_owned()),
                    None => Value::Blank,
                });
            }
            cells.push(if row.former {
                Value::Text("left the course".to_owned())
            } else {
                Value::Blank
            });
            cells
        })
        .collect();
    Table { header, rows }
}

pub fn workbook(table: &Table) -> Result<Vec<u8>, XlsxError> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("Gradebook")?;
    let bold = Format::new().set_bold();
    let points = Format::new().set_num_format("0.00");
    for (col, title) in table.header.iter().enumerate() {
        sheet.write_string_with_format(0, column(col)?, title, &bold)?;
    }
    for (row, cells) in table.rows.iter().enumerate() {
        let row = u32::try_from(row + 1).map_err(|_| XlsxError::RowColumnLimitError)?;
        for (col, value) in cells.iter().enumerate() {
            let col = column(col)?;
            match value {
                Value::Blank => {}
                Value::Text(text) => {
                    sheet.write_string(row, col, text)?;
                }
                Value::Number(number) => {
                    // A score has at most two decimals, so the text always parses.
                    let number = number.to_string().parse::<f64>().unwrap_or_default();
                    sheet.write_number_with_format(row, col, number, &points)?;
                }
            }
        }
    }
    sheet.set_freeze_panes(1, 2)?;
    sheet.autofit();
    book.save_to_buffer()
}

fn column(index: usize) -> Result<u16, XlsxError> {
    u16::try_from(index).map_err(|_| XlsxError::RowColumnLimitError)
}

#[cfg(test)]
mod tests;
