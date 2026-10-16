use std::fmt;

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub database: String,
}

/// One rejected field of a `422` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// RFC 9457 Problem Details (`application/problem+json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Per-field validation failures (ADR-0018).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Teacher,
    Student,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Web,
    App,
    Cli,
    Pat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LoginClient {
    /// The session token is delivered as an HttpOnly cookie.
    Web,
    /// The session token is returned in the response body, for use as a bearer token.
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub status: UserStatus,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[garde(length(min = 1, max = 254))]
    pub email: String,
    #[garde(length(min = 1, max = 1024))]
    pub password: String,
    #[garde(skip)]
    pub client: LoginClient,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

/// Response to a login with `client = app`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AppLoginResponse {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub user: User,
}

impl fmt::Debug for AppLoginResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppLoginResponse")
            .field("expires_at", &self.expires_at)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// `client = web` returns the user (the token is in the cookie); `client = app` returns the token too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum LoginResponse {
    App(AppLoginResponse),
    Web(User),
}

#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct ChangePasswordRequest {
    #[garde(length(min = 1, max = 1024))]
    pub current_password: String,
    #[garde(length(max = 1024))]
    pub new_password: String,
}

impl fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangePasswordRequest")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: Uuid,
    pub kind: SessionKind,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// True for the session that made this request.
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateUserRequest {
    #[garde(length(min = 1, max = 254))]
    pub email: String,
    #[garde(length(chars, min = 1, max = 100))]
    pub display_name: String,
    #[garde(skip)]
    pub role: Role,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreateUserResponse {
    pub user: User,
    /// Shown once. The user must change it at first login.
    pub temporary_password: String,
}

impl fmt::Debug for CreateUserResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateUserResponse")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TemporaryPasswordResponse {
    /// Shown once. The user must change it at next login.
    pub temporary_password: String,
}

impl fmt::Debug for TemporaryPasswordResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TemporaryPasswordResponse")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateUserRequest {
    #[garde(inner(length(chars, min = 1, max = 100)))]
    pub display_name: Option<String>,
    #[garde(skip)]
    pub role: Option<Role>,
    #[garde(skip)]
    pub status: Option<UserStatus>,
}

/// Cursor pagination (ADR-0018). The cursor is opaque: pass back `next_cursor` unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsersQuery {
    #[garde(skip)]
    pub role: Option<Role>,
    #[garde(skip)]
    pub status: Option<UserStatus>,
    /// Case-insensitive substring of the email.
    #[garde(inner(length(min = 1, max = 254)))]
    pub email: Option<String>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass as `cursor` to get the next page; `null` on the last page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CourseKind {
    Regular,
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CourseStatus {
    /// Visible to staff and admins only.
    Draft,
    Published,
    Archived,
}

/// A user's role inside one course. The owner is a `teacher` flagged `is_owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CourseRole {
    Teacher,
    Assistant,
    Student,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Course {
    pub id: Uuid,
    pub kind: CourseKind,
    pub title: String,
    pub status: CourseStatus,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateCourseRequest {
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PutMemberRequest {
    #[garde(skip)]
    pub role: CourseRole,
}

/// One member of a course. Only ids: names and emails are fetched through the user endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Membership {
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub role: CourseRole,
    pub is_owner: bool,
}
