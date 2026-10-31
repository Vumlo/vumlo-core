"use client";

import { useQuery } from "../../query";
import { fileInfo } from "../../files";
import type { FileRef } from "../types";

/** The name, type and size of a stored file; `null` for an item that has none. */
export function useFileInfo(id: string | null) {
  return useQuery<FileRef>(id ? `/files/${id}/info` : null, () => fileInfo(id!));
}
