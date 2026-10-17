"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type { Session } from "../types";
import { fetchAll } from "./util";

export function useSessions() {
  return useQuery<Session[]>("/me/sessions", () => fetchAll<Session>("/me/sessions"));
}

export function useRevokeSessions() {
  return useMutation(async (ids: string[]) => {
    for (const id of ids) await api.delete(`/me/sessions/${id}`);
    invalidate("/me/sessions");
  });
}
