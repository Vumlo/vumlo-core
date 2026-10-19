# 0061. Desktop native extras: keychain, tray, deadline notifications, deep links

Status: Accepted
Amends: ADR-0056

## Context
ADR-0056 chose Rust for the network and the session. The first release also wants what a desktop app is
for: it keeps the session safely, stays in the tray, tells a student about a deadline, and opens from a link.
It is meant to stay small, and every one of these is an operating-system integration that differs per OS.

## Decision
1. **The session token is in the operating system's keychain** (`keyring` 4, its `v1` feature: Keychain on
   macOS, Credential Manager on Windows, the Secret Service on Linux), under the service name of the app
   identifier and the account `session`. It is read once and then held in memory. There is **no file
   fallback**: a device without a keychain (a Linux desktop without a Secret Service) cannot sign in, and the
   login screen says why. A token in a plain file would be the worst copy of the session to leak.
2. **The tray** (Tauri's `tray-icon`): "Open" and "Quit", a left click brings the window back. While the
   tray exists, closing the window hides it, so the app keeps checking deadlines; if the tray cannot be
   created (no appindicator on Linux), closing quits as usual. Labels follow the window's language.
3. **One instance.** A second launch focuses the running app (`tauri-plugin-single-instance` with its
   `deep-link` feature, so a link given to the second launch reaches the first).
4. **Deadline notifications** (`tauri-plugin-notification`). The plugin cannot schedule for later, so the
   app checks every 15 minutes while it runs: it reads `/me/work` (from the server, or from the copy when
   offline), and for work that is not turned in and due within 24 hours or within an hour it shows one
   notification per deadline and window. What was shown is kept in the store (`notified`), so a restart
   does not repeat it and a moved deadline is announced again. The words are the window's language
   (`en`, `uk`). Only a student's own work is announced; the app does not guess what a teacher must grade.
5. **Deep links** `vumlo-<abbr>://...` (`tauri-plugin-deep-link`; the scheme is built from
   `institution.short_name` or `desktop.url_scheme`, ADR-0056). A link may name `home`, `courses`,
   `calendar`, `grades`, `course?id=<uuid>` or `item?course=<uuid>&id=<uuid>`. Anything else is dropped, and
   ids must look like UUIDs, so a link cannot take the window anywhere but a screen of the app. The route
   waits in the app until the window takes it (`take_route`), so a link that started the app is not lost
   before the window listens. Signed out, the person signs in first and then arrives (`?next=`).
6. **What the window may do** is granted in `capabilities/default.json`: listen to the app's own events.
   The fs, shell, http and opener plugins are not present. A probe in the real webview checked that
   `plugin:fs|read_text_file` is refused.
7. **Dependencies**: as listed in ADR-0056 (`keyring`, the three plugins), plus a pin of `rust-ini` to
   `=0.21.1` on Linux. `tauri-plugin-deep-link` 2.6.2 builds a struct that `rust-ini` 0.21.3 extended, which
   fails to compile; the pin is removed when the plugin follows.

## Consequences
- Notifications arrive only while the app is running. After "Quit" there are none; the docs say so.
- A deep link opens a build run by hand only in debug (`register_all`); an installed app is registered
  by its installer (deb, msi, dmg). An AppImage is registered by the tool that integrates it.
- On Windows a notification from an uninstalled build is shown under another application's name; installed,
  it carries the app identifier.
- Unsigned builds can be blocked from opening links or notifying by the operating system's own rules; the
  release docs say what to expect (ADR on release, later).
