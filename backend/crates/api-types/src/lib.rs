use std::fmt;

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

mod admin;
mod assessment;
mod courses;
mod document;
mod gradebook;
mod groups;
mod home;
mod items;
mod org;

pub use admin::*;
pub use assessment::*;
pub use courses::*;
pub use document::*;
pub use gradebook::*;
pub use groups::*;
pub use home::*;
pub use items::*;
pub use org::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub database: String,
    /// The release of the server binary. `vumlo-cli doctor` compares it with its own.
    pub version: String,
    /// The optional modules compiled into the server, enabled or not.
    pub modules: Vec<String>,
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
    pub name: String,
    pub surname: String,
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

/// Self-registration (ADR-0046). The account is created as a `student` and cannot log in until the
/// address is verified.
#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct RegisterRequest {
    #[garde(length(chars, min = 1, max = 100))]
    pub name: String,
    #[garde(length(chars, min = 1, max = 100))]
    pub surname: String,
    /// Its domain must be one the institution allows.
    #[garde(length(min = 1, max = 254))]
    pub email: String,
    #[garde(length(max = 1024))]
    pub password: String,
}

impl fmt::Debug for RegisterRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterRequest").finish_non_exhaustive()
    }
}

/// The token from the verification mail.
#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct VerifyEmailRequest {
    #[garde(length(min = 1, max = 200))]
    pub token: String,
}

impl fmt::Debug for VerifyEmailRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyEmailRequest").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PasswordResetRequest {
    #[garde(length(min = 1, max = 254))]
    pub email: String,
}

/// The token from the reset mail and the password to set.
#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PasswordResetConfirmRequest {
    #[garde(length(min = 1, max = 200))]
    pub token: String,
    #[garde(length(max = 1024))]
    pub new_password: String,
}

impl fmt::Debug for PasswordResetConfirmRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordResetConfirmRequest")
            .finish_non_exhaustive()
    }
}

/// Starts sign-in with Google (ADR-0047).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct GoogleStartRequest {
    /// An address to pre-fill on Google's page.
    #[garde(inner(length(max = 254)))]
    pub login_hint: Option<String>,
}

/// Where to send the browser for Google's consent screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GoogleStartResponse {
    pub authorization_url: String,
}

/// What Google sent back to the web app's callback page.
#[derive(Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct GoogleCallbackRequest {
    #[garde(length(min = 1, max = 2048))]
    pub code: String,
    #[garde(length(min = 1, max = 512))]
    pub state: String,
}

impl fmt::Debug for GoogleCallbackRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleCallbackRequest")
            .finish_non_exhaustive()
    }
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
    pub name: String,
    #[garde(length(chars, min = 1, max = 100))]
    pub surname: String,
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
    pub name: Option<String>,
    #[garde(inner(length(chars, min = 1, max = 100)))]
    pub surname: Option<String>,
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
    /// The term the course belongs to, if any (ADR-0032).
    pub term_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateCourseRequest {
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
    /// An existing term; an unknown one is a `422` on this field.
    #[serde(default)]
    #[garde(skip)]
    pub term_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PutMemberRequest {
    #[garde(skip)]
    pub role: CourseRole,
}

/// The new owner of a course (ADR-0037): an active teacher or admin.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct TransferOwnershipRequest {
    #[garde(skip)]
    pub user_id: Uuid,
}

/// One member of a course. Only ids: names and emails are fetched through the user endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Membership {
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub role: CourseRole,
    pub is_owner: bool,
    /// `direct` for a membership written on the course, `group` for a student who comes from a
    /// linked group or subgroup (ADR-0033).
    pub source: MemberSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContentStatus {
    /// Visible to staff and admins only.
    Draft,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Text,
    Assignment,
}

/// A top-level module or a sub-module (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Module {
    pub id: Uuid,
    pub course_id: Uuid,
    /// `null` for a top-level module.
    pub parent_id: Option<Uuid>,
    /// Starts at 1, among the siblings with the same parent. Gaps are possible.
    pub position: i32,
    pub title: String,
    pub status: ContentStatus,
    /// Students see the module only from this moment on.
    pub publish_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The assignment part of an item of kind `assignment` (ADR-0021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Assignment {
    /// The assignment type: `text`, `no_submission`, `quiz` or `form`. It cannot change.
    #[serde(rename = "type")]
    pub assignment_type: String,
    /// Settings of the type; an empty object for `no_submission`, the editor narrowing for `text`,
    /// the questions for a `quiz` (ADR-0050) and the fields for a `form` (ADR-0051). A student
    /// never receives the answer key of a quiz. A form's fields carry the read-only facts
    /// `has_answers` and, on the options of an exclusive field, `taken` and `taken_by`.
    #[schema(value_type = Object)]
    pub config: Map<String, Value>,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    /// Extra credit: outside the sum that the `hundred` grading scale limits (ADR-0022).
    pub is_bonus: bool,
    /// The deadline (ADR-0035). `null` when there is none, and always `null` for a type that takes
    /// no submissions.
    pub due_at: Option<DateTime<Utc>>,
    /// What happens to a draft at `due_at`; set exactly when `due_at` is.
    pub on_deadline: Option<OnDeadline>,
    /// Whether work is taken after `due_at`; set exactly when `due_at` is.
    pub accept_late: Option<bool>,
    /// The end of the late window; `null` means never. Only with `accept_late: true`.
    pub late_until: Option<DateTime<Utc>>,
    /// The most attempts a student may submit; `null` means no limit.
    pub max_attempts: Option<i32>,
    /// The day an in-class (`no_submission`) assignment takes place, `YYYY-MM-DD` (ADR-0049);
    /// `null` for any other type.
    pub class_on: Option<chrono::NaiveDate>,
}

/// What the course tree shows of an assignment: everything but `config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AssignmentSummary {
    #[serde(rename = "type")]
    pub assignment_type: String,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    pub is_bonus: bool,
    pub due_at: Option<DateTime<Utc>>,
    pub on_deadline: Option<OnDeadline>,
    pub accept_late: Option<bool>,
    pub late_until: Option<DateTime<Utc>>,
    pub max_attempts: Option<i32>,
}

/// A `text` item or an assignment. `body` is a JSON object the server stores without interpreting it;
/// for an assignment it holds the instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Item {
    pub id: Uuid,
    pub module_id: Uuid,
    pub course_id: Uuid,
    pub position: i32,
    pub kind: ItemKind,
    pub title: String,
    /// A block document (ADR-0025). The server returns the stored body as is.
    #[schema(value_type = Document)]
    pub body: Map<String, Value>,
    pub status: ContentStatus,
    pub publish_at: Option<DateTime<Utc>>,
    /// Present for items of kind `assignment`, absent for `text` items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<Assignment>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An item in the course tree: everything but the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ItemSummary {
    pub id: Uuid,
    pub kind: ItemKind,
    pub title: String,
    pub status: ContentStatus,
    pub publish_at: Option<DateTime<Utc>>,
    pub position: i32,
    /// Present for items of kind `assignment`, absent for `text` items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<AssignmentSummary>,
}

/// A sub-module with its items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContentSubModule {
    #[serde(flatten)]
    pub module: Module,
    pub items: Vec<ItemSummary>,
}

/// A top-level module with its sub-modules and its direct items. Both lists are in position
/// order; clients show sub-modules first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContentModule {
    #[serde(flatten)]
    pub module: Module,
    pub submodules: Vec<ContentSubModule>,
    pub items: Vec<ItemSummary>,
}

/// The whole content of a course as the caller may see it. Not paged: a course tree is bounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContentTree {
    pub modules: Vec<ContentModule>,
}

/// Absent field -> `None` (leave as is); `null` -> `Some(None)` (clear); a value -> `Some(Some)`.
fn set_or_clear<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateModuleRequest {
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
    /// A top-level module of this course, to create a sub-module. Omit for a top-level module.
    #[garde(skip)]
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateModuleRequest {
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
    /// Omit to keep, `null` to clear, a timestamp to set.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    #[garde(skip)]
    pub publish_at: Option<Option<DateTime<Utc>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PublishModuleRequest {
    /// Required. `true` also publishes the module's sub-modules and every draft item below it.
    #[garde(skip)]
    pub cascade: bool,
}

/// The complete ordered list of one sequence: the top-level modules, or the sub-modules of one
/// parent.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct ReorderModulesRequest {
    /// `null` or absent: reorder the top-level modules. Otherwise the sub-modules of that module.
    #[serde(default)]
    #[garde(skip)]
    pub parent_id: Option<Uuid>,
    #[garde(skip)]
    pub ids: Vec<Uuid>,
}

/// The complete ordered list of the items of one module.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct ReorderItemsRequest {
    #[garde(skip)]
    pub ids: Vec<Uuid>,
}

/// A point value is a decimal string. Anything else that is not a string is turned into the
/// empty string, which the server rejects on the field with a fixed message: serde's own message
/// would echo the value, and the field would be lost.
fn points_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => text,
        _ => String::new(),
    })
}

/// Like [`points_text`], but `null` means absent, as in the other PATCH fields.
fn optional_points_text<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => None,
        Value::String(text) => Some(text),
        _ => Some(String::new()),
    })
}

/// The assignment part of a new item of kind `assignment` (ADR-0021).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewAssignment {
    /// The assignment type: `text`, `no_submission`, `quiz` or `form`.
    #[serde(rename = "type", default)]
    pub assignment_type: String,
    /// Settings of the type, a JSON object. Defaults to `{}`; `no_submission` has none, a `quiz`
    /// needs its questions (ADR-0050) and a `form` its fields (ADR-0051).
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub config: Option<Map<String, Value>>,
    /// A decimal string with at most two decimals, greater than 0 and at most 9999.99, e.g.
    /// `"20"`, `"7.5"` or `"20.00"`. A JSON number is rejected.
    #[serde(default, deserialize_with = "points_text")]
    pub max_points: String,
    /// Extra credit, outside the sum that the `hundred` grading scale limits. Defaults to `false`.
    #[serde(default)]
    pub is_bonus: bool,
    /// The deadline (ADR-0035). With it, `on_deadline` and `accept_late` are required; without it
    /// neither may be set. Not allowed for `no_submission`.
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    /// `keep_draft` or `auto_submit` (the latter takes effect with the background worker).
    #[serde(default)]
    pub on_deadline: Option<String>,
    #[serde(default)]
    pub accept_late: Option<bool>,
    /// The end of the late window; omit for no end. Needs `accept_late: true` and must be after
    /// `due_at`.
    #[serde(default)]
    pub late_until: Option<DateTime<Utc>>,
    /// At least 1; omit for unlimited attempts.
    #[serde(default)]
    pub max_attempts: Option<i64>,
    /// The day an in-class assignment takes place, `YYYY-MM-DD` (ADR-0049). Only for
    /// `no_submission`.
    #[serde(default)]
    pub class_on: Option<chrono::NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateItemRequest {
    #[garde(skip)]
    pub kind: ItemKind,
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
    /// A block document (ADR-0025). Defaults to the empty document. For an assignment it is the
    /// description. It is read as a JSON value so that a rejection can name the path.
    #[serde(default)]
    #[schema(value_type = Option<Document>)]
    #[garde(skip)]
    pub body: Option<Value>,
    /// Required for `kind: assignment`, not allowed for `kind: text`.
    #[serde(default)]
    #[garde(skip)]
    pub assignment: Option<NewAssignment>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateItemRequest {
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
    /// A block document that replaces the body. Omit (or send `null`) to keep it.
    #[schema(value_type = Option<Document>)]
    #[garde(skip)]
    pub body: Option<Value>,
    /// Omit to keep, `null` to clear, a timestamp to set.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    #[garde(skip)]
    pub publish_at: Option<Option<DateTime<Utc>>>,
    /// Assignments only: replaces the config. Omit to keep it.
    #[schema(value_type = Option<Object>)]
    #[garde(skip)]
    pub config: Option<Map<String, Value>>,
    /// Assignments only: a decimal string, see `NewAssignment`. Omit to keep it.
    #[serde(default, deserialize_with = "optional_points_text")]
    #[garde(skip)]
    pub max_points: Option<String>,
    /// Assignments only. Omit to keep it.
    #[garde(skip)]
    pub is_bonus: Option<bool>,
    /// Assignments only (ADR-0035). Omit to keep, `null` to clear, a timestamp to set. The rules
    /// are checked on the resulting settings: to remove a deadline send `due_at`, `on_deadline`
    /// and `accept_late` as `null` together.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    #[garde(skip)]
    pub due_at: Option<Option<DateTime<Utc>>>,
    /// Assignments only. Omit to keep, `null` to clear, `keep_draft` or `auto_submit` to set.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<String>)]
    #[garde(skip)]
    pub on_deadline: Option<Option<String>>,
    /// Assignments only. Omit to keep, `null` to clear.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<bool>)]
    #[garde(skip)]
    pub accept_late: Option<Option<bool>>,
    /// Assignments only. Omit to keep, `null` to clear.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    #[garde(skip)]
    pub late_until: Option<Option<DateTime<Utc>>>,
    /// Assignments only. Omit to keep, `null` for unlimited attempts, a number of at least 1 to set.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<i64>)]
    #[garde(skip)]
    pub max_attempts: Option<Option<i64>>,
    /// `no_submission` assignments only (ADR-0049). Omit to keep, `null` to clear, a day
    /// (`YYYY-MM-DD`) to set.
    #[serde(default, deserialize_with = "set_or_clear")]
    #[schema(value_type = Option<chrono::NaiveDate>)]
    #[garde(skip)]
    pub class_on: Option<Option<chrono::NaiveDate>>,
}

#[cfg(test)]
mod content_tests;
