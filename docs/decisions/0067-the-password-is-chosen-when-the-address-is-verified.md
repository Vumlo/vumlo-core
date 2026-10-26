# 0067. The password is chosen when the address is verified

Status: Accepted
Amends: ADR-0046 (decisions 2, 3 and 6)

## Context
ADR-0046 let `POST /auth/register` take the password, and a second registration of an unverified
address replaced the name, surname and password. A verification link proves control of the mailbox,
not who chose the password, so under that flow the password of the account came from whoever
registered last, not necessarily from the person who verifies the address. The password should come
from the person who proves the mailbox.

## Options
1. **Set the password at verification.** `register` takes no password. `POST /auth/verify-email` takes
   `{token, password}`, so the person who proves the mailbox is the person who chooses the password.
   Cost: a password step on the page behind the link.
2. **Require the registered password at verification.** The newest registration still wins and the
   verification checks the password it set. Cost: `password` becomes required in a request that exists
   (a breaking change, ADR-0012), and the person retypes it.
3. **Bind to the registering browser** with a secret cookie, as sign-in with Google does. Cost: the
   link works only in the browser that registered, which breaks mail apps that open links elsewhere,
   and the desktop app has no browser to bind to.

## Decision
Option 1.
1. **Registration.** `POST /auth/register {name, surname, email}` creates the unverified `student`
   without a password identity. A `password` in the body is still accepted, so an older client keeps
   working, and is ignored. The domain and name checks, the uniform `202`, the per-address throttle
   and the event of ADR-0046 are unchanged. A hash of a random value is still computed and thrown
   away, so the time does not depend on whether the address has an account. Registering an unverified
   address again replaces the name and surname and voids the unused verify tokens, as before.
2. **Verification.** `POST /auth/verify-email {token, password}`: a missing, spent, expired or
   disabled-account token is `410`, decided before the password is looked at; then the password is
   required (`422` on `password` without it) and checked against the policy of ADR-0017 with the
   owner's own data as context (`422` on `password`), both leaving the link usable; then the hash,
   before the transaction; then one transaction that consumes the token (`410` if a concurrent request
   won), sets the password, and sets `email_verified_at`. The `password` field of the request is
   optional in the schema so that the document stays additive, but every valid link needs it.
3. **A verified address is not changed by a verification link.** If the account was verified another
   way since the link was minted (a reset link, or Google), the token is `410`: only a reset changes the
   password of a verified account.
4. **An old password does not count.** A password identity that an unverified account holds from before
   this ADR is replaced when the address is verified, so a password set before the address was verified
   never survives.
5. **Login.** An account that has not been verified has no password, so a login on it is `401` like any
   wrong password. ADR-0046 decision 6 (`403` for a correct password on an unverified account) no
   longer applies to accounts made by registration.
6. **Web.** The page behind the link shows the password form of the reset page. The registration form
   no longer asks for a password. The mail says that the link also lets the owner choose a password.

## Consequences
- The account of a registration has no credential until its address is verified, and then it has the
  one its owner chose. Registering an address again can still make a pending registration unusable
  (the old link is void) and sets the name that is shown until the owner registers again; the per-IP
  and per-address limits bound both.
- A client built for ADR-0046 that sends only `{token}` to `verify-email` now gets `422`. The web app
  and the mock API are changed with this ADR; the desktop app loads the same pages.
- Registration costs the same work as before: the throttles are unchanged, and the pad hash keeps the
  time per request the same.
