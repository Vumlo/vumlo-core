"use client";

import { api } from "../client";
import { useQuery } from "../../query";
import type { Institution } from "../types";

/** The public profile; also served before sign-in (ADR-0044). */
export function useInstitution() {
  return useQuery<Institution>("/institution", () => api.get<Institution>("/institution"));
}
