"use client";

import { api } from "../client";
import { useQuery } from "../../query";
import type { Institution } from "../gaps";

/** GAP: no institution endpoint yet (API_GAPS.md, "Institution"). */
export function useInstitution() {
  return useQuery<Institution>("/institution", () => api.get<Institution>("/institution"));
}
