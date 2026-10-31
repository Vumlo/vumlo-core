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
  http.get(`${A}/files/:id`, ({ request, params }) => {
    requireUser();
    const file = getDb().files.get(String(params.id));
    if (!file) throw notFound();
    const inline = INLINE.has(file.mime);
    // One `bytes=a-b` or `bytes=a-` range, as the server serves it: a screen reads a file's size from `Content-Range`.
    const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.get("range") ?? "");
    const size = file.bytes.byteLength;
    const first = range ? Number(range[1]) : 0;
    const last = range ? Math.min(range[2] ? Number(range[2]) : size - 1, size - 1) : size - 1;
    if (range && first >= size) return new HttpResponse(null, { status: 416, headers: { "Content-Range": `bytes */${size}` } });
    return new HttpResponse(range ? file.bytes.slice(first, last + 1) : file.bytes, {
      status: range ? 206 : 200,
      headers: {
        ...(range ? { "Content-Range": `bytes ${first}-${last}/${size}` } : {}),
        "Content-Type": file.mime,
        "Content-Disposition": `${inline ? "inline" : "attachment"}; filename="${file.name.replace(/[^\x20-\x7e]|["\\;%]/g, "_")}"; filename*=UTF-8''${encodeURIComponent(file.name)}`,
        "X-Content-Type-Options": "nosniff",
        "Content-Security-Policy": "sandbox",
        ETag: `"${params.id}"`,
      },
    });
  }),
];
