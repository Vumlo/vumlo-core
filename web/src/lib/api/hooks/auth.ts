"use client";

import { api } from "../client";
import { clearQueries, invalidate, useMutation, useQuery } from "../../query";
import type { User } from "../../session";
import type { GoogleHandoff, GoogleStart, MockMail } from "../gaps";

/** The server answers `202` with no body for every address (ADR-0046); `pnpm dev:mock` answers the same. */
export function useRegister() {
  return useMutation(async (input: { name: string; surname: string; email: string; password: string }) => {
    await api.post("/auth/register", input);
    invalidate("/mock/outbox");
    return true as const;
  });
}

export function useVerifyEmail() {
  return useMutation(async (token: string) => {
    await api.post("/auth/verify-email", { token });
    return true as const;
  });
}

export function useRequestPasswordReset() {
  return useMutation(async (email: string) => {
    await api.post("/auth/password-reset", { email });
    invalidate("/mock/outbox");
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

/** Mock only: the mail the fake server "sent". There is no such endpoint on a real server. */
export function useMockOutbox(enabled: boolean) {
  return useQuery<{ items: MockMail[] }>(enabled ? "/mock/outbox" : null, () => api.get<{ items: MockMail[] }>("/mock/outbox"));
}
