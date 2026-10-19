use chrono::{DateTime, Datelike, NaiveDate, Utc};
use uuid::Uuid;

const NAME_MAX_LEN: usize = 100;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {NAME_MAX_LEN} characters")]
    Name,
    #[error("must not be before starts_on")]
    Dates,
    #[error("must be a date in the years 1 to 9999")]
    StartsOnRange,
    #[error("must be a date in the years 1 to 9999")]
    EndsOnRange,
    #[error(
        "must be an https address of at most {DOWNLOAD_URL_MAX_LEN} characters, without spaces"
    )]
    DownloadUrl,
    #[error("each platform may appear once")]
    DownloadDuplicate,
}

const DOWNLOAD_URL_MAX_LEN: usize = 2048;

/// The address of an installer: `https`, a host, no whitespace or control character (ADR-0064). Anything
/// that is not that could be a `javascript:` link or a page that is not served over TLS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadUrl(String);

impl DownloadUrl {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let url = raw.trim();
        let host = url
            .strip_prefix("https://")
            .and_then(|rest| rest.split(['/', '?', '#']).next());
        let host_ok = host.is_some_and(|host| {
            let name = host.rsplit_once('@').map_or(host, |(_, name)| name);
            !host.contains('@') && !name.is_empty() && !name.starts_with(['.', ':'])
        });
        if host_ok
            && url.len() <= DOWNLOAD_URL_MAX_LEN
            && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            Ok(Self(url.to_owned()))
        } else {
            Err(ModelError::DownloadUrl)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
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
        // PostgreSQL stores dates from 4713 BC, but a term in year 0 or 40 000 is a typo, and the
        // part of chrono's range below the database's is an error 500 (ADR-0039).
        let years = 1..=9999;
        if !years.contains(&starts_on.year()) {
            return Err(ModelError::StartsOnRange);
        }
        if !years.contains(&ends_on.year()) {
            return Err(ModelError::EndsOnRange);
        }
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

/// A person in a group, as the administrator's overview lists them (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberLine {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    pub added_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A group, its people and the courses that use it (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupOverview {
    pub group_id: Uuid,
    pub members: Vec<MemberLine>,
    pub linked_courses: Vec<crate::teaching::LinkedCourseRow>,
}

/// How many regular courses use a term, and where today stands against it (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermUsage {
    pub term_id: Uuid,
    pub courses: i64,
    pub current: bool,
    pub ended: bool,
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
mod tests;
