import type { components } from "./schema";

type S = components["schemas"];

export type User = S["User"];
export type Role = S["Role"];
export type Course = S["Course"];
export type CourseRole = S["CourseRole"];
export type Term = S["Term"];
export type ItemRevision = S["ItemRevision"];
export type GradeHistoryEntry = S["GradeHistoryEntry"];
/** What the server sends per item: `class_on` is a day (`YYYY-MM-DD`). */
export type ServerItemMeta = S["ItemMeta"];
/**
 * `GET /courses/{id}/item-meta` per item as the screens use it: `class_on` is an instant, noon of
 * the class day in the institution's time zone, so a date never slips to the day before.
 */
export type ItemMeta = Omit<S["ItemMeta"], "class_on"> & { class_on?: string | null };
export type ItemMetaMap = { items: Record<string, ItemMeta> };
export type UserSummary = S["UserSummary"];
export type UserFacts = S["UserFacts"];
export type GroupOverview = S["GroupOverview"];
export type TermStat = S["TermStat"];
export type AdminCourse = S["AdminCourse"];
export type AuditRecord = S["AuditRecord"];
export type AuditKind = S["AuditKind"];
export type CourseCard = S["CourseCard"];
export type CardTodo = S["CardTodo"];
export type CardRole = S["CardRole"];
export type MyWork = S["MyWork"];
export type MyWorkItem = S["WorkItem"];
export type WorkStatus = S["WorkStatus"];
export type TeachingOverview = S["TeachingOverview"];
export type SearchHit = S["SearchHit"];
export type CourseInfo = S["CourseInfo"];
export type TemplateSummary = S["TemplateSummary"];
export type PersonRow = S["PersonRow"];
export type LinkedGroup = S["LinkedGroup"];
export type CoursePeople = S["CoursePeople"];
export type Candidate = S["Candidate"];
export type Institution = S["Institution"];
export type AdminInstitution = S["AdminInstitution"];
export type Group = S["Group"];
export type Subgroup = S["Subgroup"];
export type GroupOption = S["GroupOption"];
export type GroupMember = S["GroupMember"];
export type CourseGroupLink = S["CourseGroupLink"];
export type Membership = S["Membership"];
export type Module = S["Module"];
type Cfg = Record<string, unknown>;
export type Assignment = Omit<S["Assignment"], "config"> & { config: Cfg };
/** The generated `config` is `Record<string, never>`; a type's config is any JSON object. */
export type Item = Omit<S["Item"], "assignment"> & { assignment?: Assignment };
export type ItemSummary = S["ItemSummary"];
export type AssignmentSummary = S["AssignmentSummary"];
export type ContentTree = S["ContentTree"];
export type ContentModule = S["ContentModule"];
export type ContentSubModule = S["ContentSubModule"];
export type ContentStatus = S["ContentStatus"];
export type MyGrades = S["MyGrades"];
export type MyGrade = S["MyGrade"];
export type MySubmission = S["MySubmission"];
export type Attempt = S["Attempt"];
export type SubmissionRow = S["SubmissionRow"];
export type GradebookPage = S["GradebookPage"];
export type GradebookColumn = S["GradebookColumn"];
export type GradebookRow = S["GradebookRow"];
export type GradebookCell = S["GradebookCell"];
export type Session = S["Session"];
export type EditorProfiles = S["EditorProfiles"];
export type CreateUserResponse = S["CreateUserResponse"];

// Block documents (ADR-0025, ADR-0027). The generated union keeps blocks as anonymous objects,
// so the pieces the renderer and the editor use are named here.
export type Inline = S["Inline"];
export type Styles = S["Styles"];
export type CodeLanguage = S["CodeLanguage"];
export type Document = S["Document"];
export type Block = Document["blocks"][number];
export type BlockType = Block["type"];
export type Feature = S["Feature"];
export type ProfileName = S["ProfileName"];

export function emptyDocument(): Document {
  return { v: 1, blocks: [] };
}

/** The body of an item as a block document (ADR-0025); a missing one is empty. */
export function bodyOf(item: Pick<Item, "body">): Document {
  const body = item.body as Partial<Document> | undefined;
  return { v: 1, blocks: Array.isArray(body?.blocks) ? body.blocks : [] };
}
