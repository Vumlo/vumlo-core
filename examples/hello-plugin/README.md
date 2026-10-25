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

`vumlo-cli plugin disable hello` stops the events and refuses the token at once.

`python3 plugin.py --check-vector` checks its signature code against the vector the core's tests use.
