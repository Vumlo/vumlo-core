# The desktop app

A desktop app for your students and teachers, for Linux, Windows and macOS. It is the Vumlo web app in a
native window that reads without a connection, keeps what a student writes until there is one, tells them
a day and an hour before work is due, and opens from links. It belongs to **one** institution: it is built
from your `vumlo.toml` and talks to your server only. You build it; Vumlo does not publish installers.

What it does, and does not:

- **Reads offline.** Courses, materials, deadlines, grades and the calendar that the person has opened or the
  app has fetched. It tells them when it shows a saved copy, and when that copy is from.
- **Drafts and submits wait for the connection.** A document assignment and a form are saved on the device and
  sent when the server answers, in order, once. The server has the last word: a deadline that passed while the
  device was offline is refused, and the app shows the server's words and keeps the text to copy.
- **Quizzes need a connection.** They are timed and counted by the server.
- **Adding a file needs a connection.** A file picked in the editor or on a hand-in page is sent at once and is
  refused offline; it is not queued like a draft. Opening a file saves it to Downloads (see [Files](#files)).
- **Notifications only while the app runs.** It stays in the tray when the window is closed. "Quit" in the tray
  stops them.
- **No automatic updates.** Publish a new installer; people install it over the old one.

## What you need on the server

Release 0.2 or newer: the desktop app uses `Idempotency-Key`, `ETag` and the app sign-in with Google (ADR-0058,
ADR-0059, ADR-0062). `server.public_origin` must be the address people will reach (`https://...`): it is
compiled into the app, and the app sends passwords there.

## Make it yours

Everything comes from `vumlo.toml`, as for the web app, and `vumlo-cli brand apply`:

| Key | What it becomes |
| --- | --- |
| `server.public_origin` | The server the app talks to (compiled in). The identifier of the app is its host, reversed: `lms.ucu.edu.ua` is `ua.edu.ucu.lms`. |
| `institution.short_name` | The name of the app and of its shortcut (`UCU`; a short name that is not in Latin letters gives way to the scheme, `vumlo-uku`, because a Debian package can only be named in ASCII), and, with `vumlo-`, the URL scheme of its links, `vumlo-ucu://...`, and the name of the installers, `vumlo-ucu_0.2.2_amd64.deb`. |
| `institution.name` | The publisher of the installer and its description (the full name can be long, and in any alphabet). |
| `institution.logo` | The icon of the app, in every size each system wants. Without it the app has Vumlo's placeholder icon. |
| `desktop.url_scheme` (optional) | The scheme of the links, `vumlo-` and letters, digits and hyphens. **Set it if your short name has no Latin letters**; otherwise it is plain `vumlo`, and two institutions' apps on one computer would fight over `vumlo://`. |

Sign-in with Google needs nothing more than the web app does (`[auth.google]`, ADR-0047): the same redirect
address. The app opens Google in the person's browser and comes back through a link.

## Build it on your computer

You need Node 24, a Rust toolchain (stable) and, per system:

- **Linux:** `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libgtk-3-dev patchelf`.
- **Windows:** Microsoft C++ Build Tools and WebView2 (in Windows 11).
- **macOS:** Xcode command line tools.

An installer is built on the system it is for. From the directory of this release (or of your repository, if
core is vendored: `vendor/vumlo-core`):

```sh
./vumlo brand apply                         # writes .vumlo/generated next to vumlo.toml
cd web && npm ci && cd ../desktop && npm ci
node scripts/build.mjs -- --bundles deb,appimage      # Linux; nsis on Windows, dmg on macOS
```

`scripts/build.mjs` builds the web app as a static export, makes the icons from your logo, writes the
configuration of the installer (`desktop/gen/`) and runs `tauri build`. The installers are in
`desktop/dist/`, named `<scheme>_<version>_<arch>.<ext>`, for example `vumlo-ucu_0.2.1_amd64.deb`. It warns, before the long part, if the server address is `http` or
`localhost`, or the scheme is plain `vumlo`. If your configuration is not beside `desktop/`, give the
generated directory: `node scripts/build.mjs --generated /path/to/.vumlo/generated`.

## Build it with GitHub Actions

Copy `desktop/release/desktop-release.yml` to `.github/workflows/desktop-release.yml` of the repository that has
your `vumlo.toml` (a GitHub fork of vumlo-core, or a repository that vendors a release as `vendor/vumlo-core`),
and set `CORE_DIR` and `CONFIG` at the top. Push a tag:

```sh
git tag desktop-v1.0.0 && git push origin desktop-v1.0.0
```

The workflow makes the branding once (or uses a committed `.vumlo/generated`), builds the installers on Linux,
Windows and macOS (Apple silicon and Intel), and puts them with a `SHA256SUMS` in a **draft** release. Open it,
try an installer, and publish. "Run workflow" builds them without a release.

## Signing

Without secrets the installers are **unsigned**. They work; the system warns:

- **Windows:** SmartScreen says "Windows protected your PC". "More info", then "Run anyway". The warning fades
  as an unsigned installer is downloaded by many people; a certificate removes it.
- **macOS:** Gatekeeper refuses to open it ("cannot be opened because the developer cannot be verified"). Open
  it with a right click, "Open", or with `xattr -dr com.apple.quarantine /Applications/<name>.app`. This is the
  documented way, not a workaround you can hide from your users: unsigned apps are for testing and small groups.
- **Linux:** nothing to sign. A `.deb` or AppImage opens as any other.

To sign, add these repository secrets; the workflow uses what is there and says in the release what it did.

| System | Secrets | Result |
| --- | --- | --- |
| Windows | `WINDOWS_CERTIFICATE` (the `.pfx`, base64), `WINDOWS_CERTIFICATE_PASSWORD` | The installer and the program are signed. A code-signing certificate from a certificate authority is paid. |
| macOS | `APPLE_CERTIFICATE` (the Developer ID Application `.p12`, base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | Signed. |
| macOS, to notarize | also `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password), `APPLE_TEAM_ID` | Notarized: opens without a question. Needs an Apple Developer account. |

## Give it to people

Publish the draft release. Then, in the web app as an administrator, open **Admin, Institution, Desktop app** and
paste the address of each installer (`https://...`; any of the five can stay empty). From then on:

- students and teachers on a Windows, Mac or Linux computer see a notice on their home page with a button for
  their system, and can hide it;
- there is a download page, `https://<your site>/download`, that needs no sign-in: put that address on your
  university's own pages;
- the account menu has "Get the desktop app".

Save all the fields empty and the notice, the page and the menu entry are gone. The links are public, since the
download page is.

Which file goes in which field: `...-setup.exe` is Windows; `...-aarch64.dmg` is a Mac with Apple silicon and
`...-x64.dmg` an Intel Mac; `.deb` is Ubuntu and Debian and `.AppImage` other Linux. Use the address of each file
on the published release (right click it, "copy link address"), or wherever you host them, and keep `SHA256SUMS`.
A new version is a new tag, `desktop-v1.0.1`, and new files: paste the new addresses, save, and people see them
at once. They install over the old version and keep their data.

## Files

The window can upload a file and open one (ADR-0069, ADR-0061). It has no access to the disk of its own; two
commands of the app do the work, and the window asks for them by file id:

- **Upload.** The file the person picks is sent by the app to the server with their session; the limit is the
  server's `files.max_size`. The whole file is held in memory while it is sent.
- **Open.** The app downloads the file, saves it in the system's **Downloads** folder (a name already taken
  gets a number: `report (1).pdf`; a file is never overwritten) and then:
  - opens it in the program of the system if it is a document, spreadsheet, presentation, text, image,
    audio or video file (`pdf doc docx odt rtf txt md ppt pptx odp xls xlsx ods csv png jpg jpeg gif webp mp3
    wav mp4 webm mov`, judged by the last extension of the name);
  - only **shows it in its folder** if it is anything else. That includes archives (`zip`, `7z`, `tar`, `gz`;
    the system would unpack them) and every program or script (`.exe`, `.sh`, `.bat`, ...). The app never
    opens or runs those. If the system cannot show the folder, the file stays in Downloads.
- **Where the file came from is recorded** so the system can warn before it is opened by something else:
  Windows gets a `Zone.Identifier` stream (the "downloaded from the Internet" mark, best effort), and the macOS
  app sets `LSFileQuarantineEnabled` so the system quarantines files the app creates. Linux has no such mark.
  **The Windows and macOS parts were not run on those systems in the testing for this release**; they follow the
  system documentation and the tests of the code that runs on Linux. Try them with your own installer before you
  rely on them.
- **The `vumlo-file` scheme** (`vumlo-file://localhost/<id>`) is internal: it lets the window show an image or
  a PDF without holding the token. It serves one file by id, only to the app's own page, and is not something
  to link to or configure.

The desktop app needs a server of the release that has file storage; against an older server the file
screens answer with an error.

## Limits to know about

- **One institution per build.** The server is compiled in. To serve two, build two apps.
- **Linux needs a keychain.** The session is kept in the system keychain (GNOME Keyring, KWallet or another Secret
  Service), never in a file. On a desktop without one, signing in says so.
- **Links** (`vumlo-ucu://course?id=...`) need the app installed and registered by its installer. A bare AppImage
  is registered by the tool that integrates AppImages, and its desktop file cannot pass the link on: on Linux, sign-in
  with Google needs the `.deb` (sign-in with a password works in both).
- **What is on the device.** What the person read and what they have not sent are in one file in the app's data
  folder (`~/.local/share/<identifier>`, `%APPDATA%\<identifier>`, `~/Library/Application Support/<identifier>`),
  not encrypted: disk encryption of the device protects it. Signing out deletes it.
- **Old servers.** An app built against 0.2 needs a 0.2 server. The API only grows, so an app keeps working
  with a newer server; build a new one when you want its new features.
