"use client";

import { api } from "../client";
import { clearQueries, useMutation } from "../../query";
import type { User } from "../../session";
import type { GoogleHandoff, GoogleStart } from "../gaps";

/** The server answers `202` with no body for every address (ADR-0046); `pnpm dev:mock` answers the same. The password is chosen later, on the page behind the link (ADR-0067). */
export function useRegister() {
  return useMutation(async (input: { name: string; surname: string; email: string }) => {
    await api.post("/auth/register", input);
    return true as const;
  });
}

export function useVerifyEmail() {
  return useMutation(async (token: string, password: string) => {
    await api.post("/auth/verify-email", { token, password });
    return true as const;
  });
}

export function useRequestPasswordReset() {
  return useMutation(async (email: string) => {
    await api.post("/auth/password-reset", { email });
    return true as const;
  });
}

export function useConfirmPasswordReset() {
  return useMutation(async (token: string, new_password: string) => {
    await api.post("/auth/password-reset/confirm", { token, new_password });
    return true as const;
  });
}

export function useGoogleStart() {
  return useMutation((loginHint: string) => api.post<GoogleStart>("/auth/google/start", { login_hint: loginHint || undefined }));
}

export function useGoogleCallback() {
  return useMutation(async (code: string, state: string, hint: string | null) => {
    const answer = await api.post<User | GoogleHandoff>("/auth/google/callback", { code, state, hint: hint ?? undefined });
    // A sign-in the desktop app started ends in the app, not in this browser: there is no session here.
    if ("handoff" in answer) return answer;
    clearQueries();
    return answer;
  });
}
