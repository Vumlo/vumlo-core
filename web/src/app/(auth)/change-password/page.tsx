"use client";

import { ChangePasswordForm, type ChangePasswordWhere } from "@/components/ChangePasswordForm";

const where: ChangePasswordWhere = {
  signIn: "/login",
  landing: (user, next) => (next && next.startsWith("/") && !next.startsWith("//") ? next : user.role === "admin" ? "/courses" : "/"),
};

export default function ChangePasswordPage() {
  return <ChangePasswordForm where={where} />;
}
