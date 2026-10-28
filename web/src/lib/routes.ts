// Every id is in the query string, not in the path (ADR-0057): a static export cannot list the
// courses of an installation, and one scheme serves the web app and the desktop app.
const q = encodeURIComponent;

export type CourseTab = "grades" | "people" | "settings";

export const courseHref = (courseId: string, tab?: CourseTab) => `/course${tab ? `/${tab}` : ""}?id=${q(courseId)}`;
/** The grading workspace of one assignment; `studentId` opens that student's submission, else the first one is shown. */
export const gradeHref = (courseId: string, assignmentId: string, studentId?: string) => `/course/grade?id=${q(courseId)}&item=${q(assignmentId)}${studentId ? `&student=${q(studentId)}` : ""}`;
/** `tab`: the id of a plugin whose page is shown in the tab of an assignment instead of the assignment (ADR-0066). */
export const itemHref = (courseId: string, itemId: string, tab?: string) => `/course/item?id=${q(courseId)}&item=${q(itemId)}${tab ? `&tab=${q(tab)}` : ""}`;
export const editItemHref = (courseId: string, itemId: string) => `/course/item/edit?id=${q(courseId)}&item=${q(itemId)}`;
export const moduleHref = (name: string, path: string[] = []) => `/m?name=${q(name)}${path.length ? `&path=${path.map(q).join("/")}` : ""}`;
