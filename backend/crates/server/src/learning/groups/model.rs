use uuid::Uuid;

use crate::learning::teaching::LinkedCourseRow;
use org::MemberLine;

/// A group, its people and the courses that use it (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupOverview {
    pub group_id: Uuid,
    pub members: Vec<MemberLine>,
    pub linked_courses: Vec<LinkedCourseRow>,
}
