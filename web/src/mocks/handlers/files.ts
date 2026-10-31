import { http, HttpResponse } from "msw";
import { getDb, uid } from "../db";
import { canView, courseById, itemById, notFound, problem, requireStaff, requireUser } from "../logic";

const A = "/api/v1";

// What a browser may show in place; everything else is saved (the server's rule, root AGENTS.md section 6).
const INLINE = new Set(["application/pdf", "image/png", "image/jpeg", "image/gif", "image/webp", "text/plain"]);
let count = 0;

/** The mock trusts the type the client sent; the server finds it from the bytes. */
export const fileHandlers = [
  http.post(`${A}/files`, async ({ request }) => {
    const user = requireUser();
    const url = new URL(request.url);
    const [kind, id] = (url.searchParams.get("for") ?? "").split(":");
    if ((kind !== "item" && kind !== "assignment") || !id) throw problem(422, "Unprocessable", undefined, [{ field: "for", message: "for must be item:{id} or assignment:{id}" }]);
    const item = itemById(id);
    const course = courseById(item.course_id);
    if (kind === "item") requireStaff(course.id, user);
    else if (!item.assignment || !canView(user, course)) throw notFound();
    const name = (url.searchParams.get("name") ?? "").split(/[\\/]/).pop()?.trim().slice(0, 200) ?? "";
    if (!name) throw problem(422, "Unprocessable", undefined, [{ field: "name", message: "the file needs a name" }]);
    const bytes = await request.arrayBuffer();
    if (bytes.byteLength === 0) throw problem(422, "Unprocessable", undefined, [{ field: "file", message: "the file is empty" }]);
    const db = getDb();
    if (bytes.byteLength > db.fileMax) throw problem(413, "Payload Too Large");
    const mime = request.headers.get("content-type") || "application/octet-stream";
    const fileId = uid(`file:${count++}:${name}`);
    db.files.set(fileId, { name, mime, bytes });
    return HttpResponse.json({ id: fileId, name, mime, size: bytes.byteLength }, { status: 201 });
  }),
  http.get(`${A}/files/:id`, ({ params }) => {
    requireUser();
    const file = getDb().files.get(String(params.id));
    if (!file) throw notFound();
    const inline = INLINE.has(file.mime);
    return new HttpResponse(file.bytes, {
      headers: {
        "Content-Type": file.mime,
        "Content-Disposition": `${inline ? "inline" : "attachment"}; filename*=UTF-8''${encodeURIComponent(file.name)}`,
        "X-Content-Type-Options": "nosniff",
        "Content-Security-Policy": "sandbox",
        ETag: `"${params.id}"`,
      },
    });
  }),
];
