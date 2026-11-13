"use client";

import { api } from "../client";
import { clearQueries, useMutation } from "../../query";
import type { User } from "../../session";

/** Signs an administrator in to the admin console (ADR-0095): its own cookie, its own origin. */
export function useConsoleLogin() {
  return useMutation(async (email: string, password: string) => {
    const user = await api.post<User>("/console/login", { email, password });
    clearQueries();
    return user;
  });
}
