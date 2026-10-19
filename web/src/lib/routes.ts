// Every id is in the query string, not in the path (ADR-0057): a static export cannot list the
// courses of an installation, and one scheme serves the web app and the desktop app.
const q = encodeURIComponent;

export type CourseTab = "grades" | "people" | "settings";

export const courseHref = (courseId: string, tab?: CourseTab) => `/course${tab ? `/${tab}` : ""}?id=${q(courseId)}`;
export const itemHref = (courseId: string, itemId: string) => `/course/item?id=${q(courseId)}&item=${q(itemId)}`;
export const editItemHref = (courseId: string, itemId: string) => `/course/item/edit?id=${q(courseId)}&item=${q(itemId)}`;
export const moduleHref = (name: string, path: string[] = []) => `/m?name=${q(name)}${path.length ? `&path=${path.map(q).join("/")}` : ""}`;
