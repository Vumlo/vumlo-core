//! Request and response types of the course screens (ADR-0048): the info header, templates, the
//! people of a course and the search for people to add. Points are decimal strings with two
//! decimals (ADR-0021).

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{Course, CourseRole, MemberSource, Role};

/// The header of every course screen. A read model: nothing here is stored as it is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseInfo {
    pub course_id: Uuid,
    /// The short code, e.g. `CS 305`; empty when none is set.
    pub code: String,
    pub description: String,
    /// The name of the course's term; `null` without a term.
    pub term_name: Option<String>,
    pub owner_name: String,
    pub owner_surname: String,
    /// The owner is the person to ask, so everyone who can view the course gets this address.
    pub owner_email: String,
    /// Current students: on the roster and active.
    pub student_count: u32,
    /// People who left the roster but have results in the course (ADR-0037).
    pub former_count: u32,
    pub assistant_count: u32,
    /// When the course row (title, term, code, description, status, owner) last changed.
    pub updated_at: DateTime<Utc>,
    /// When a module or item last changed; `null` for a course with no content.
    pub content_updated_at: Option<DateTime<Utc>>,
    /// The sum of `max_points` of the regular assignments the caller may see.
    pub total_points: String,
    /// How many assignments that sum counts.
    pub graded_items: u32,
    /// A grade has a value in the course, so it cannot be deleted, only archived.
    pub has_grades: bool,
}

/// Only the fields present change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCourseInfoRequest {
    /// At most 32 characters after trimming; empty clears the code.
    #[garde(inner(length(chars, max = 32)))]
    pub code: Option<String>,
    /// At most 300 characters after trimming; empty clears the description.
    #[garde(inner(length(chars, max = 300)))]
    pub description: Option<String>,
}

/// Makes a template from a course.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct MakeTemplateRequest {
    /// The template's title; the course's own when absent.
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
}

/// Makes a draft course from a template.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateFromTemplateRequest {
    /// The new course's title; the template's own when absent.
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
    /// An existing term; an unknown one is a `422` on this field.
    #[garde(skip)]
    pub term_id: Option<Uuid>,
}

/// A template with what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TemplateSummary {
    pub course: Course,
    /// Modules and sub-modules.
    pub sections: u32,
    pub lectures: u32,
    pub assignments: u32,
    pub updated_at: DateTime<Utc>,
}

/// One person on a course.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PersonRow {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Staff only: students never get another person's address.
    pub email: Option<String>,
    pub role: CourseRole,
    pub is_owner: bool,
    pub source: MemberSource,
    /// For a student who comes from a linked group: the group's name, and the subgroup's after a
    /// slash when the link is to a subgroup.
    pub group: Option<String>,
    /// Off the roster but with results in the course. Staff only (ADR-0037).
    pub former: bool,
}

/// A group or subgroup linked to the course. Staff only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LinkedGroup {
    pub link_id: Uuid,
    pub group_name: String,
    /// Set when the link is to one subgroup.
    pub subgroup_name: Option<String>,
    /// Current students the link brings (active, not excluded).
    pub student_count: u32,
    pub linked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CoursePeople {
    pub people: Vec<PersonRow>,
    /// Empty for a student.
    pub groups: Vec<LinkedGroup>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CandidatesQuery {
    /// Part of a name, surname or email: at least 2 characters.
    #[garde(length(chars, min = 2, max = 100))]
    pub q: String,
}

/// Someone who could be added to the course.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Candidate {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    /// The institution role.
    pub role: Role,
}

/// At most 20, best order first. Not paged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Candidates {
    pub items: Vec<Candidate>,
}
