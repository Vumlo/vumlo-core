"use client";

import { ChangePasswordForm, type ChangePasswordWhere } from "@/components/ChangePasswordForm";

const where: ChangePasswordWhere = {
  signIn: "/console/login",
  // Only a page of the console: this door never leads into the student app.
  landing: (_user, next) => (next && (next === "/console" || next.startsWith("/console/")) ? next : "/console"),
};

export default function ConsoleChangePasswordPage() {
  return <ChangePasswordForm where={where} />;
}
