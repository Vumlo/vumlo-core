# 0046. Self-registration, email verification and password reset

Status: Accepted
Supplements: ADR-0017, ADR-0003, ADR-0038, ADR-0045

## Context
ADR-0017 plans two account paths the server does not have: self-registration with a password and a
verification link, and password reset by mail. It fixes the lifetimes and the rule that the responses
do not reveal whether an address has an account. It leaves open how the tokens are stored, what a
second registration of an unverified address does, and what the endpoints look like.

A verification link proves that someone controls the mailbox, not that they chose the password. If
the first registration of an address won, a stranger could register a victim's address with their own
password, and the victim would later verify an account the stranger can log into.

## Options
- Registering an address that has an unverified account: (a) ignore the request: the stranger's
  password stays; (b) **the newest registration replaces the name, surname and password and voids the
  unused verification tokens**: only the owner of the mailbox can verify, and the password is the one
  they chose last.
- Tokens: (a) stored plain in a table: a database read yields working links; (b) **a random 256-bit
  value of which only the SHA-256 is stored**, minted by the worker (ADR-0045) so the event keeps ids
  only (ADR-0007) and the raw value exists only in the mail.
- Existing accounts: (a) leave `email_verified_at` null and make everyone verify: locks out every
  account; (b) **a migration sets it for existing users**, and admin and CLI created users are
  verified when created (the admin vouched for the address).

## Decision
1. **Table.** `identity.email_tokens (id, user_id, purpose verify | reset, token_hash, created_at,
   expires_at, used_at)`, `token_hash` unique. A verify token lives 24 hours, a reset token 1 hour,
   both single use. Minting a token voids the unused ones of the same user and purpose.
2. **Registration.** `POST /auth/register {name, surname, email, password}` answers `202` with no body
   whatever the address is. The request is validated first: the email's domain must equal one of
   `auth.allowed_domains` (case-insensitive; an empty list allows nothing), and the password must pass
   the policy of ADR-0017 with the person's own data as context (`422` on `password`). The password is
   always hashed, before any transaction, so the time does not depend on the address.
   In one transaction: no user with the address, so create a `student` with `email_verified_at` null,
   `must_change_password` false and a password identity, and emit `user.verification_requested`; an
   unverified, active student, so replace the name, surname and password, void the unused verify
   tokens and emit the event again; any other account (verified, disabled, another role), so do
   nothing. Self-registration is not audited (ADR-0034, point 2: a person acting on their own account).
3. **Verification.** `POST /auth/verify-email {token}` consumes the token with one conditional
   `UPDATE ... WHERE used_at IS NULL AND expires_at > now()` and sets `email_verified_at`. An unknown,
   malformed, used or expired token is `410`. It does not sign anyone in.
4. **Reset request.** `POST /auth/password-reset {email}` answers `202` with no body for every address.
   For an active user (verified or not, with a password or only a Google identity) it emits
   `user.password_reset_requested`; for a disabled or unknown address nothing happens.
5. **Reset confirmation.** `POST /auth/password-reset/confirm {token, new_password}`: a missing or spent
   token is `410` before the password is looked at; then the policy (`422` on `new_password`) with the
   owner's data as context; then the hash, before the transaction; then one transaction that consumes
   the token (`410` if a concurrent request won), sets the password, sets `email_verified_at` when
   null (the link proved the mailbox), clears `must_change_password`, voids the user's other reset
   tokens and deletes every session of the user. A disabled account's token is `410`.
6. **Login.** A correct password on an account whose address is not verified is `403`
   ("email address is not verified"), after the password check, so the response does not tell a stranger
   anything about an address. The per-account counter is reset as for any correct password.
7. **Limits.** The routes of this ADR sit behind their own per-IP limiter (same settings as the login
   limiter, ADR-0038) and the two that send mail also count per address in a second in-memory throttle
   (`auth.login_account_max_attempts` per `auth.login_account_window_minutes`), keyed by the hash of the
   address whether or not it exists, so mail cannot be used to flood one mailbox and the answer
   stays uniform. A refused request is `429`.
8. **Events.** `user.verification_requested` and `user.password_reset_requested`, version 1, payload
   `{user_id}`. The `identity` handler of ADR-0045 loads the user, skips the event if it no longer
   applies (already verified, disabled), mints the token in its own transaction and sends the mail
   after the commit. The link is `<mail.link_base>/verify-email?token=...` or `/reset-password?token=...`.
9. **Mail off.** Without `[mail]`, `register` and `password-reset` are `404`. `verify-email` and
   `password-reset/confirm` keep working.

## Consequences
- An attacker who registers a victim's address gets no working account: the victim's later
  registration replaces the password, and the attacker cannot read the mail.
- Someone can still make a victim's pending registration unusable by registering again (the old link
  is void). The victim registers again. Accepted: the per-IP and per-address limits bound it.
- A person who registered and never verified keeps an inactive row; a cleanup job comes with the
  worker's scheduled jobs (ADR-0007).
- Registration answers do not reveal whether the address exists, but the time of the DB branch differs
  slightly; the hash, the dominant cost, is the same.
- Existing users are all treated as verified by the migration.
