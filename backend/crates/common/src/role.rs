use crate::text_enum;

/// A user's role in the institution (ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Teacher,
    Student,
}
text_enum!(Role { Admin => "admin", Teacher => "teacher", Student => "student" });

#[cfg(test)]
mod tests;
