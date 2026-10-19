"use client";

import { api, isDesktop } from "./api/client";
import { clearQueries, invalidate, useMutation, useQuery } from "./query";
import type { components } from "./api/schema";

export type User = components["schemas"]["User"];
export type Role = User["role"];

/** The signed-in user, or an ApiError 401 when nobody is signed in. */
export function useMe() {
  return useQuery<User>("/me", () => api.get<User>("/me"));
}

export function useLogin() {
  return useMutation(async (email: string, password: string) => {
    const user = await api.post<User>("/auth/login", { email, password, client: isDesktop() ? "app" : "web" });
    clearQueries();
    return user;
  });
}

export function useLogout() {
  return useMutation(async () => {
    await api.post("/auth/logout");
    clearQueries();
  });
}

export function useChangePassword() {
  return useMutation(async (current_password: string, new_password: string) => {
    await api.post("/me/password", { current_password, new_password });
    temporary = "";
    invalidate("/me");
    return true as const;
  });
}

let temporary = "";

/** Kept in memory only, so the page that asks for a new password can reuse what was just typed. */
export function rememberTemporaryPassword(password: string) {
  temporary = password;
}

export function temporaryPassword() {
  return temporary;
}

/** Where a person lands after signing in: the page they asked for, or their home. */
export function landingPath(user: Pick<User, "role" | "must_change_password">, next: string | null) {
  const target = next && next.startsWith("/") && !next.startsWith("//") ? next : user.role === "admin" ? "/admin" : "/";
  return user.must_change_password ? `/change-password?next=${encodeURIComponent(target)}` : target;
}

export function fullName(u: Pick<User, "name" | "surname">) {
  return `${u.name} ${u.surname}`;
}
