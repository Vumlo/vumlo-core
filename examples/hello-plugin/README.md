# hello-plugin

The smallest plugin of Vumlo ([ADR-0066](../../docs/decisions/0066-modules-and-plugins.md)): a Python
file with no dependencies. For every submitted work it counts the words and posts a report that
staff read at `GET /api/v1/submissions/{id}/plugin-reports`.

1. Set `VUMLO__PLUGINS__SECRET_KEY` (32 characters or more) where the server and `vumlo-cli` run.
2. Start the plugin with no credentials, so the core can read its manifest:
   `PORT=9000 python3 plugin.py`
3. Install it and approve what it asks for:
   `vumlo-cli plugin install http://127.0.0.1:9000 --grant submissions:read,reports:write`
   It prints a token and a secret, once.
4. Stop the plugin and start it with them:
   `PORT=9000 VUMLO_API=http://127.0.0.1:8080/api/v1 VUMLO_PLUGIN_TOKEN=... VUMLO_WEBHOOK_SECRET=... python3 plugin.py`
5. A student submits; within a few seconds the report is there.

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
