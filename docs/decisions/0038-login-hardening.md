# 0038. Login hardening: sessions against a verified credential, counted attempts, bounded hashing

Status: Accepted
Amends: ADR-0017

## Context
The audit of task 014 found four weaknesses around login, all small to fix and all real:

- A-03: a login checks the password (about 30 ms), and only then writes the session. A password reset,
  a password change or a disabling that ran in between deleted the sessions that existed, not the one
  that was about to be written. The old password then bought a session that outlived the reset.
- A-04: the per-account limit (10 per 15 minutes) was checked before the hash and counted after it, so
  40 parallel guesses got 37 evaluations.
- A-06: the request log held the full URI. The user list filters by email in the query string, so
  addresses were written to the log, also for requests that were not signed in.
- A-09: the per-client limiter kept one entry per address forever (tower_governor expects the caller to
  clean it up), counted every IPv6 address of a /64 as a client of its own, and nothing bounded how
  many Argon2 hashes ran at once, each taking about 19 MiB. The limits were constants.

## Decision
1. **A session is created against what was verified.** The transaction that inserts the session
   first locks the user and the password identity `FOR SHARE` and reads them again: the account must
   be active and the hash must be the one that was checked, otherwise the login fails like any other.
   A reset, a password change or a disabling then either has committed before (the login is refused)
   or waits for this commit and deletes the new session with the others.
2. **An attempt is counted when it starts.** `LoginThrottle::reserve` takes one of the account's
   attempts before the hash and refuses with `429` once the window's attempts are used; a success
   resets the count. A parallel burst gets at most `login_account_max_attempts` evaluations. The
   same limit counts wrong current passwords on a password change. When the table of counters is full
   it drops expired entries first, then accounts that are not locked out, oldest first, so spraying
   random addresses cannot free a locked account. A window that ends while a refusal is being worked
   out is a new window (it was a panic).
3. **The request log carries the method and the path, not the query string.**
4. **The per-client limiter.** A client is an IPv4 address, or the /64 of an IPv6 address
   (an IPv4-mapped address is the IPv4 one). Entries whose allowance is full again are forgotten
   once a minute. The client address is still the header named by `server.client_ip_header`, else the
   TCP peer.
5. **Hashing is bounded.** At most `auth.max_concurrent_hashes` hashes and checks run at once in a
   server process; the rest wait for a permit (they are also bounded by the request timeout). The
   CLI and the tests use the ungated function.
6. **The limits are configuration**, under `[auth]`, each with an environment override
   (`LEARNLY__AUTH__…`), range-checked at startup:

   | key | default | meaning |
   |---|---|---|
   | `login_ip_burst` | 50 | login attempts one client may make in a burst |
   | `login_ip_replenish_seconds` | 1 | seconds until one attempt is given back |
   | `login_account_max_attempts` | 10 | attempts per account per window |
   | `login_account_window_minutes` | 15 | the window |
   | `max_concurrent_hashes` | 8 | hashes and checks at once (about 19 MiB each) |

   A school or a lecture hall behind one address raises `login_ip_burst` and lowers
   `login_ip_replenish_seconds`; without `server.client_ip_header` behind a reverse proxy every
   client is the proxy.
7. **Tokio features.** `tokio` gets the features `sync` (the semaphore) and `time`; both were
   already compiled in through sqlx and tower, and are now declared where they are used.

## Consequences
- The 429 of the per-account limit no longer waits for the hash, so it also costs the server less.
- An attacker can still lock a known account for one window (ADR-0017 accepted that); the limit now
  holds under parallel use, and a legitimate login during a flood may be refused like the flood.
- A login takes two more locks on the account and the identity row for the length of its short
  transaction.
- The limiter state is per process (a single binary, ADR-0001); two instances double the limits.
