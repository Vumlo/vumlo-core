# hello-plugin

The smallest plugin of Vumlo ([ADR-0066](../../docs/decisions/done.md)): a Python
file with no dependencies. For every submitted work it counts the words and posts a report that
staff read at `GET /api/v1/submissions/{id}/plugin-reports`. It also keeps a running total on the assignment
(`PUT /plugin-api/assignments/{id}/reports/totals`, read at `GET /api/v1/assignments/{id}/plugin-reports`):
a report is kept under a key, on an attempt, an assignment or a course, and writing it again replaces it
([ADR-0077](../../docs/decisions/done.md)).

1. Set `VUMLO__PLUGINS__SECRET_KEY` (32 characters or more) where the server and `vumlo-cli` run.
2. Start the plugin with no credentials, so the core can read its manifest:
   `PORT=9000 python3 plugin.py`
3. Install it and approve what it asks for:
   `vumlo-cli plugin install http://127.0.0.1:9000 --grant submissions:read,reports:write,settings:read,assignments:read --courses all`
   It prints a token and a secret, once. `--courses` says which courses the plugin may reach: `all`, or the
   ids of some courses, comma-separated ([ADR-0074](../../docs/decisions/done.md)). Every call the plugin
   makes is checked against it, and it is sent the events of those courses only.
4. Stop the plugin and start it with them:
   `PORT=9000 VUMLO_API=http://127.0.0.1:8080/api/v1 VUMLO_PLUGIN_TOKEN=... VUMLO_WEBHOOK_SECRET=... python3 plugin.py`
5. A student submits; within a few seconds the report is there. When it was granted `assignments:read` the
   report also says the type of the assignment (`GET /plugin-api/assignments/{id}`); without it the plugin
   gets a `403` for that call and reports the words alone.

It has two settings, declared in its manifest (`MANIFEST["settings"]` in `plugin.py`), which an administrator
changes without touching the plugin:

    vumlo-cli plugin settings list hello
    vumlo-cli plugin settings set hello unit characters                    # the whole institution
    vumlo-cli plugin settings set hello max_words 300 --course <course id> # one course
    vumlo-cli plugin settings unset hello unit                             # back to the default

The plugin reads them with `GET /plugin-api/settings?course_id=...` (scope `settings:read`) and drops what it
read when the core sends `plugin.settings.changed`; see `settings_for` and `do_POST`. A secret (a token the
plugin needs for another service) is declared `{"type": "string", "secret": true}`, set with
`vumlo-cli plugin settings set <plugin> <key> -` (the value comes from standard input), shown by no command, and
returned only to the plugin, in that same response.

It also has a page. Every assignment gets a tab **Words** (the address gains `&tab=hello`); a teacher sees how
many words were submitted for it in all, a student is told the numbers are for teachers. How it works, for a
plugin of your own:

- The page is shown in a frame that has no cookie and no access to the web app, so it cannot ask the API
  anything as the person looking. It sends `{type: "vumlo.ready", v: 1}` to its parent and gets back
  `vumlo.context`: the assignment, a `launch` token, the `theme`, the `language` and `tokens`, the design
  tokens of the web app, which it sets on its `:root` and draws with (`var(--text)`, `var(--accent)`, ...).
- Its own server learns who is looking from the launch token, which the core signed with the webhook secret:
  `payload.signature`, where `signature` is the HMAC-SHA256 of `"vumlo-launch." + payload`. See
  `launch_claims`. Check the signature, `exp` and `plugin`; never trust the assignment or the role the page
  *says*, only the token.
- The page sends `vumlo.resize` with its height. The frame's origin is `null`, so the page's server answers
  its calls with CORS; the page sends its own strict CSP (this one hashes its script and style).
- Set `VUMLO_ORIGIN` to the origin of the web app so that nobody else can frame the page, and
  `PUBLIC_URL` if the core reaches the plugin by an address other than `http://127.0.0.1:PORT`.

`vumlo-cli plugin disable hello` stops the events and refuses the token at once; the tab is gone the next time
the web app loads its list of pages (a tab that is still open can no longer get a launch token).

`python3 plugin.py --check-vector` checks its signature and launch-token code against the vectors the core's tests use.

## Routes for people

The manifest also declares three `routes`: `GET /api/summary/{course_id}` (teachers and administrators; and in the course
only its teacher or assistants) `POST /api/echo` (anyone signed in) and `GET /api/feed/{feed}` (anyone, see below). A person reaches them through the core at
`/api/v1/plugins/hello/api/summary/<course id>` with their ordinary session ([ADR-0078](../../docs/decisions/done.md)).
The plugin never sees the session; the core tells it who the person is in headers it signs with the webhook secret:

    X-Vumlo-User, X-Vumlo-Role, X-Vumlo-Course, X-Vumlo-Course-Role, X-Vumlo-Timestamp, X-Vumlo-Nonce, X-Vumlo-Signature

`check_proxy` in `plugin.py` is the whole check: the signature (it covers the user, the role, the course, the
request-target and the body), a timestamp within five minutes, and a nonce it has not seen. A plugin that skips any of
the three can be fooled by someone who sends its port a request of their own, or by a captured call sent again. What
the plugin answers must be JSON, text, CSV, iCalendar, PDF or XLSX: the core serves it from its own origin and passes
on nothing that could run in a browser.

### A public route

`GET /api/feed/{feed}?token=` declares `"auth": "none"` ([ADR-0080](../../docs/decisions/done.md)): a calendar app can
fetch it with a URL alone, so there is no session. The core still signs the call, but as one that nobody made as a
person: it sends `X-Vumlo-Anonymous: 1` and no user, role or course, and the signature has `none` where a person's
call has `session` (`check_proxy` returns `user: None`). What the core cannot know is whether the caller may have this
feed, so the plugin checks its own token (`feed_token`: only this server can make one) and answers `404` for any other.
The core limits how often a public route may be called, for everyone together, so a leaked URL cannot be turned into a
flood against the plugin.
