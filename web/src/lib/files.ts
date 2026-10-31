import { convertFileSrc } from "@tauri-apps/api/core";
import { ApiError, isDesktop, readHeaders, readText, requestRaw } from "@/lib/api/client";
import type { FileRef } from "@/lib/api/types";
import { clientConfig } from "@/lib/clientConfig";
import { translate } from "@/lib/i18n/core";
import { notice } from "@/lib/notice";

export type { FileRef };

/** What a file is for: an item of a course (a teacher's material) or an assignment (a student's work). */
export type FileTarget = { item: string } | { assignment: string };

/**
 * Where `<img>` and `<iframe>` get a stored file from: the API in a browser, the app's `vumlo-file` protocol on the
 * desktop. Tauri writes that address (it differs between Windows and the other systems); the id is the path.
 */
export const fileUrl = (id: string): string => (isDesktop() ? convertFileSrc(id, "vumlo-file") : `/api/v1/files/${encodeURIComponent(id)}`);

/** Stores a file for `target`. A file over the institution's limit fails here, before any byte is sent. */
export async function uploadFile(file: File, target: FileTarget): Promise<FileRef> {
  const max: number | undefined = clientConfig.public.files?.max_size;
  if (max !== undefined && file.size > max) throw new ApiError(413, "Payload Too Large", translate("files.tooLarge"));
  const owner = "item" in target ? `item:${target.item}` : `assignment:${target.assignment}`;
  const path = `/files?for=${encodeURIComponent(owner)}&name=${encodeURIComponent(file.name)}`;
  try {
    return await requestRaw<FileRef>("POST", path, file, { "Content-Type": file.type || "application/octet-stream" });
  } catch (e) {
    if (!(e instanceof ApiError)) throw new ApiError(0, "upload", translate("files.uploadFailed"));
    if (e.status === 413) throw new ApiError(413, e.title, translate("files.tooLarge"));
    if (e.status === 422 && e.errors.length === 0) throw new ApiError(422, e.title, translate("files.typeNotAllowed"));
    throw e;
  }
}

/**
 * Shows a file to the person. A PDF opens in a tab of its own; any other file is downloaded under its name.
 * The desktop app saves it and opens it in the program of the system, then says what it did.
 */
export async function openFile(ref: FileRef): Promise<void> {
  if (isDesktop()) {
    const { invoke } = await import("@tauri-apps/api/core");
    const outcome = await invoke<"opened" | "revealed" | "saved_only">("open_file", { id: ref.id });
    notice(translate(outcome === "opened" ? "files.opened" : "files.savedToDownloads", { name: ref.name }));
  } else if (ref.mime === "application/pdf") {
    window.open(fileUrl(ref.id), "_blank", "noopener");
  } else {
    const a = document.createElement("a");
    a.href = fileUrl(ref.id);
    a.download = ref.name;
    document.body.append(a);
    a.click();
    a.remove();
  }
}

/** The first `maxBytes` of a text file. */
export const readFileText = (id: string, maxBytes: number): Promise<string> => readText(fileUrl(id), maxBytes);

/**
 * The name, type and size of a stored file, from the headers of its first byte (the server has no endpoint for them).
 * The desktop scheme does not hand the page `Content-Disposition`, so there the name is empty and a screen says the item's title.
 */
export async function fileInfo(id: string): Promise<FileRef> {
  const h = await readHeaders(fileUrl(id));
  const size = Number(/\/(\d+)$/.exec(h.get("content-range") ?? "")?.[1] ?? h.get("content-length") ?? 0);
  const disposition = h.get("content-disposition") ?? "";
  const encoded = /filename\*=UTF-8''([^;]+)/i.exec(disposition)?.[1];
  let name = /filename="([^"]*)"/i.exec(disposition)?.[1] ?? "";
  try {
    if (encoded) name = decodeURIComponent(encoded);
  } catch {
    // A name that is not valid percent-encoding keeps the plain one.
  }
  return { id, name, mime: (h.get("content-type") ?? "").split(";")[0]!.trim(), size };
}

/** "512 B", "48 KB", "3.2 MB". */
export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return translate("files.sizeB", { n: bytes });
  if (bytes < 1_048_576) return translate("files.sizeKb", { n: Math.round(bytes / 1024) });
  return translate("files.sizeMb", { n: (bytes / 1_048_576).toFixed(1) });
}
