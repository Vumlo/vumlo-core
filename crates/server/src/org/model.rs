use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

const NAME_MAX_LEN: usize = 100;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {NAME_MAX_LEN} characters")]
    Name,
    #[error("must not be before starts_on")]
    Dates,
}

/// The name of a term, a group or a subgroup: trimmed, 1 to 100 characters, no control
/// characters (ADR-0032).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgName(String);

impl OrgName {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let name = raw.trim();
        let len = name.chars().count();
        if (1..=NAME_MAX_LEN).contains(&len) && !name.chars().any(char::is_control) {
            Ok(Self(name.to_owned()))
        } else {
            Err(ModelError::Name)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The span of a term. `ends_on` may equal `starts_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermDates {
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
}

impl TermDates {
    pub fn new(starts_on: NaiveDate, ends_on: NaiveDate) -> Result<Self, ModelError> {
        if ends_on >= starts_on {
            Ok(Self { starts_on, ends_on })
        } else {
            Err(ModelError::Dates)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub id: Uuid,
    pub name: String,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
}

/// The fields of a term update that were sent.
#[derive(Debug, Clone, Default)]
pub struct TermUpdate {
    pub name: Option<String>,
    pub starts_on: Option<NaiveDate>,
    pub ends_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subgroup {
    pub id: Uuid,
    pub group_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// A group a course can be linked to, with its subgroups: ids and names, no members (ADR-0033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupOption {
    pub id: Uuid,
    pub name: String,
    pub subgroups: Vec<Subgroup>,
}

/// Why one entry of a member list was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberProblem {
    NoSuchUser,
    NotAStudent,
    NotInGroup,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_bounded() {
        assert_eq!(OrgName::parse("  7-A ").unwrap().as_str(), "7-A");
        assert!(OrgName::parse("").is_err());
        assert!(OrgName::parse("   ").is_err());
        assert!(OrgName::parse("a\tb").is_err());
        assert!(OrgName::parse(&"ł".repeat(100)).is_ok());
        assert!(OrgName::parse(&"ł".repeat(101)).is_err());
    }

    #[test]
    fn a_term_may_last_one_day_but_not_end_before_it_starts() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 9, d).unwrap();
        assert!(TermDates::new(day(1), day(1)).is_ok());
        assert!(TermDates::new(day(1), day(30)).is_ok());
        assert_eq!(TermDates::new(day(2), day(1)), Err(ModelError::Dates));
    }
}
