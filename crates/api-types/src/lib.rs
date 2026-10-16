use std::fmt;

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
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
    /// The assignment type, `text` for now. It cannot change.
    #[serde(rename = "type")]
    pub assignment_type: String,
    /// Settings of the type; an empty object for `text`.
    #[schema(value_type = Object)]
    pub config: Map<String, Value>,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    /// Extra credit: outside the sum that the `hundred` grading scale limits (ADR-0022).
    pub is_bonus: bool,
}

/// What the course tree shows of an assignment: everything but `config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AssignmentSummary {
    #[serde(rename = "type")]
    pub assignment_type: String,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    pub is_bonus: bool,
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
    #[schema(value_type = Object)]
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
    /// The assignment type. Only `text` exists so far.
    #[serde(rename = "type", default)]
    pub assignment_type: String,
    /// Settings of the type, a JSON object. Defaults to `{}`; `text` has none.
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateItemRequest {
    #[garde(skip)]
    pub kind: ItemKind,
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
    /// A JSON object, stored as is. Defaults to `{}`. For an assignment it holds the instructions.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    #[garde(skip)]
    pub body: Option<Map<String, Value>>,
    /// Required for `kind: assignment`, not allowed for `kind: text`.
    #[serde(default)]
    #[garde(skip)]
    pub assignment: Option<NewAssignment>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateItemRequest {
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
    /// A JSON object that replaces the body. Omit to keep it.
    #[schema(value_type = Option<Object>)]
    #[garde(skip)]
    pub body: Option<Map<String, Value>>,
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
}

#[cfg(test)]
mod content_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn publish_at_distinguishes_absent_null_and_a_value() {
        let absent: UpdateModuleRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(absent.publish_at, None);
        let cleared: UpdateModuleRequest =
            serde_json::from_value(json!({"publish_at": null})).unwrap();
        assert_eq!(cleared.publish_at, Some(None));
        let set: UpdateItemRequest =
            serde_json::from_value(json!({"publish_at": "2030-01-02T03:04:05Z"})).unwrap();
        assert!(matches!(set.publish_at, Some(Some(_))));
    }

    #[test]
    fn cascade_is_required() {
        assert!(serde_json::from_value::<PublishModuleRequest>(json!({})).is_err());
        let request: PublishModuleRequest =
            serde_json::from_value(json!({"cascade": false})).unwrap();
        assert!(!request.cascade);
    }

    #[test]
    fn a_body_must_be_an_object() {
        for body in [json!([1]), json!("text"), json!(3)] {
            let create = json!({"kind": "text", "title": "t", "body": body});
            assert!(serde_json::from_value::<CreateItemRequest>(create).is_err());
            assert!(serde_json::from_value::<UpdateItemRequest>(json!({"body": body})).is_err());
        }
        let ok: CreateItemRequest =
            serde_json::from_value(json!({"kind": "text", "title": "t", "body": {"a": 1}}))
                .unwrap();
        assert_eq!(ok.body.unwrap()["a"], json!(1));
    }

    #[test]
    fn the_item_kinds_are_text_and_assignment() {
        assert_eq!(serde_json::to_value(ItemKind::Text).unwrap(), json!("text"));
        let old = json!({"kind": "lesson", "title": "t"});
        assert!(serde_json::from_value::<CreateItemRequest>(old).is_err());
    }

    #[test]
    fn a_tree_entry_flattens_the_module_fields() {
        let module = Module {
            id: Uuid::nil(),
            course_id: Uuid::nil(),
            parent_id: None,
            position: 1,
            title: "t".into(),
            status: ContentStatus::Draft,
            publish_at: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let value = serde_json::to_value(ContentModule {
            module,
            submodules: vec![],
            items: vec![],
        })
        .unwrap();
        assert_eq!(value["title"], json!("t"));
        assert_eq!(value["submodules"], json!([]));
    }

    fn new_assignment(value: Value) -> NewAssignment {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_new_assignment_takes_defaults_for_what_is_missing() {
        let empty = new_assignment(json!({}));
        assert_eq!(empty.assignment_type, "");
        assert_eq!(empty.max_points, "");
        assert_eq!(empty.config, None);
        assert!(!empty.is_bonus);

        let only_type = new_assignment(json!({"type": "text"}));
        assert_eq!(only_type.assignment_type, "text");
        assert_eq!(only_type.max_points, "");

        let only_points = new_assignment(json!({"max_points": "20"}));
        assert_eq!(only_points.assignment_type, "");
        assert_eq!(only_points.max_points, "20");
    }

    #[test]
    fn a_new_assignments_points_that_are_not_text_become_the_rejected_empty_string() {
        for sent in [
            json!(20),
            json!(7.5),
            json!(true),
            json!([1]),
            json!({"a": 1}),
            json!(null),
        ] {
            let assignment = new_assignment(json!({"type": "text", "max_points": sent}));
            assert_eq!(assignment.max_points, "", "{sent}");
        }
        let kept = new_assignment(json!({"type": "text", "max_points": "1.005"}));
        assert_eq!(
            kept.max_points, "1.005",
            "text is passed on for the server to judge"
        );
    }

    #[test]
    fn an_update_reads_points_text_and_treats_null_as_absent() {
        let update = |value: Value| serde_json::from_value::<UpdateItemRequest>(value).unwrap();
        assert_eq!(update(json!({})).max_points, None);
        assert_eq!(update(json!({"max_points": null})).max_points, None);
        assert_eq!(
            update(json!({"max_points": "12.5"})).max_points.as_deref(),
            Some("12.5")
        );
        for sent in [json!(12), json!(false), json!([]), json!({})] {
            assert_eq!(
                update(json!({"max_points": sent})).max_points.as_deref(),
                Some(""),
                "{sent}"
            );
        }
        let nulls = update(json!({"config": null, "is_bonus": null}));
        assert_eq!(nulls.config, None);
        assert_eq!(nulls.is_bonus, None);
        let set = update(json!({"config": {}, "is_bonus": true}));
        assert_eq!(set.config, Some(Map::new()));
        assert_eq!(set.is_bonus, Some(true));
    }

    #[test]
    fn a_text_item_serializes_without_an_assignment_key() {
        let item = |assignment| Item {
            id: Uuid::nil(),
            module_id: Uuid::nil(),
            course_id: Uuid::nil(),
            position: 1,
            kind: ItemKind::Text,
            title: "t".into(),
            body: Map::new(),
            status: ContentStatus::Draft,
            publish_at: None,
            assignment,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let text = serde_json::to_value(item(None)).unwrap();
        assert!(text.get("assignment").is_none());

        let assignment = serde_json::to_value(item(Some(Assignment {
            assignment_type: "text".into(),
            config: Map::new(),
            max_points: "20.00".into(),
            is_bonus: false,
        })))
        .unwrap();
        assert_eq!(
            assignment["assignment"],
            json!({"type": "text", "config": {}, "max_points": "20.00", "is_bonus": false})
        );
    }

    #[test]
    fn a_tree_summary_without_an_assignment_has_no_key() {
        let summary = ItemSummary {
            id: Uuid::nil(),
            kind: ItemKind::Text,
            title: "t".into(),
            status: ContentStatus::Draft,
            publish_at: None,
            position: 1,
            assignment: None,
        };
        assert!(
            serde_json::to_value(summary)
                .unwrap()
                .get("assignment")
                .is_none()
        );
    }
}
