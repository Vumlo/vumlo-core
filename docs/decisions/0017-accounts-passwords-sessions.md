# 0017. Accounts, passwords and session lifetimes

Status: Accepted
Supplements: ADR-0003

## Context
ADR-0003 restricts registration to allowed email domains. But a domain check does not prove a person owns the address: anyone could register as `rector@<domain>`. We also need rules for passwords, session lifetimes and disabling users.

## Decision

### Account creation — three paths
1. **Created by an admin** (`learnly-server create-user` / `create-admin`, or the admin API). The admin sets the role. The system generates a temporary password, shown once, and sets `must_change_password = true`. Until the password is changed, the session may only call: change password, logout, `GET /me`.
2. **Self-registration with a password.** The email must be in `allowed_domains`. The account is created with `email_verified_at = null` and **cannot log in until the email is verified** through a single-use link (token stored hashed, expires in 24 h). Requires email delivery (task 004).
3. **Self-registration with Google.** Allowed domain + `email_verified = true` from Google. The account is created already verified, with role `student`.

- Registration and password-reset responses do not reveal whether an email already exists (protection against account enumeration).
- New self-registered users get role `student`. An admin promotes teachers.

### Password policy (NIST SP 800-63B rev. 4, OWASP ASVS)
- Minimum length is configurable (`auth.password_min_length`), default **12**. Maximum 128. Spaces and Unicode are allowed.
- **No composition rules** (no "uppercase + digit + symbol").
- Strength check with `zxcvbn`: score ≥ 3. User inputs (email, display name, institution name, "learnly") are passed as context. The feedback is returned to the client for a strength meter.
- No periodic expiry. A change is forced only for temporary passwords.
- Changing the password revokes all other sessions of that user. A password reset (task 004) revokes all sessions.

### Login
- One generic error for a wrong email or wrong password.
- When the user does not exist, a dummy hash is still verified, so response time does not reveal which emails exist.
- Rate limits per IP and per account on login, registration and reset (task 002 implements login; the per-account counter also covers wrong current passwords on change-password).
- Per-IP limit: `tower_governor`. The key is the header named by `server.client_ip_header` (set it when a reverse proxy overwrites it), otherwise the TCP peer address.
- Per-account limit is in-memory (single binary), 10 failures / 15 min, reset on success; accepted trade-off: an attacker can temporarily lock out a known email.

### Session lifetimes
| kind | lifetime |
|------|----------|
| `web` | 30 days, sliding (extended on activity) |
| `app` | 90 days, sliding |
| `cli` | 90 days, absolute |
| `pat` | chosen by the user, at most 365 days, absolute |

- `last_seen_at` / expiry are updated at most once per hour per session, not on every request.
- Expired sessions are deleted by a cleanup job once the worker exists (task 003). Until then, expired sessions are simply rejected.

### Admin actions
- A password reset issues a new temporary password (`must_change_password = true`) and deletes all the user's sessions.
- The last active admin cannot be demoted or disabled (409). The check locks all active admins, so two concurrent requests cannot remove the last two.
- A new password must differ from the current one.

### Disabling a user
- `status = disabled` revokes **all** of the user's sessions in the same transaction. A disabled user cannot log in. Their data is kept.

## Consequences
- Task 002 ships only admin-created accounts and password login. Password self-registration and reset arrive with email (task 004), Google with task 005.
- New tables: `identity.email_tokens` (task 004). New columns: `users.email_verified_at`, `users.must_change_password`.
- New dependencies: `zxcvbn` (task 002) and `lettre` (task 004). See ADR-0014.
