# 0064. Download links of the desktop app, set by an admin

Status: Accepted
Amends: ADR-0044, ADR-0063

## Context
An institution that builds its desktop app (ADR-0063) puts the installers somewhere (a GitHub release, its own
file server) and wants its people to find them: a hint on the site, and a page to download from. Where the links
are is the institution's to say, and an admin should say it in the web app, not by editing a file and restarting
the server: ADR-0044 made the rest of the institution profile read-only, from the configuration.

## Options
1. **A configuration key** (`desktop.downloads` in `vumlo.toml`). Fits ADR-0044, needs no table, but needs the
   person who runs the server and a restart to change a link, and nothing to edit in the web app.
2. **A table of links in the database, edited by an admin** in the web app; the public profile lists them.
3. **A fixed pattern** (the GitHub release of the institution's repository). The server cannot know the
   repository, and the files are named after the institution.

## Decision
1. **Option 2.** A new table `org.app_downloads` (`platform` primary key, `url`, `updated_at`; migration 0031),
   one link per kind of installer: `windows`, `macos_arm`, `macos_intel`, `linux_deb`, `linux_appimage` (the
   installers of ADR-0063).
2. **Reading is public.** `GET /institution` gains `app_downloads: [{platform, url}]`, in the order above, empty
   when there is none. The page that needs the links (the sign-in page, the download page) needs no session. If
   the list cannot be read, the profile is answered without it, rather than not answered.
3. **Writing is for admins.** `PUT /admin/institution/app-downloads {downloads: [...]}` sets the **whole**
   list: a platform that is not in it is removed, an empty list switches the feature off. A new policy action
   `ManageAppDownloads` (admins only). Each change is audited as `institution.app_downloads.set` with the
   platforms that have a link, not the addresses. The installation has no id of its own, so the audit entry
   names it with the nil UUID.
4. **What a link may be.** `https`, a host, at most 2048 characters, no space or control character, no user
   information. Anything else is `422` on the field `downloads`, as is a platform twice or more than five
   entries. This keeps `javascript:` and plain-HTTP downloads out of a link the whole institution clicks.
5. **In the web app.** With at least one link:
   - a notice on the home page (students and teachers; not admins, who go to their own area) on a computer the
     app is made for: "Download for Windows" when there is exactly one link for the system, else "Get the app" to
     the page, and "All downloads"; it can be hidden (kept in the browser);
   - a page `/download`, public, that lists the installers by system with this computer's first;
   - an entry "Get the desktop app" in the account menu.

   None of these is shown in the desktop app itself, and none on a phone or a tablet (no notice). Without links
   the notice and the menu entry are not there, and the page says there is no app.
6. **The admin form** is a section "Desktop app" on the Institution page: five fields and a save button. The
   only place that writes.

## Consequences
- An institution publishes or withdraws the app without touching the server. A link that stops working is the
  institution's to fix; the server does not check that a link is reachable.
- The profile is public, so the links are public. They are links to a download, nothing secret.
- A new installer kind needs a code change (the list of platforms is a check constraint and an enum).
- `GET /institution` now reads the database. It is still answered when the read fails, without the links.
