import { passwordStrength } from "@/lib/password";

export function StrengthMeter({ password, id }: { password: string; id: string }) {
  const st = passwordStrength(password);
  return (
    <>
      <div aria-hidden="true" style={{ display: "flex", gap: 4, marginTop: 4 }}>
        {[1, 2, 3, 4].map((i) => (
          <span key={i} style={{ flex: 1, height: 4, borderRadius: 2, background: i <= st.level ? "var(--ok-fg)" : "var(--border)" }} />
        ))}
      </div>
      <p id={id} aria-live="polite" style={{ fontSize: 13, color: st.level >= 3 ? "var(--ok-fg)" : "var(--muted)", minHeight: 20 }}>{st.label}</p>
    </>
  );
}
