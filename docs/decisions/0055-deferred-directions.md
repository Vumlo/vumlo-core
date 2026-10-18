# 0055. Deferred directions: widgets, plugins, an end-user CLI, native clients

Status: Accepted
Supplements: ADR-0012, ADR-0054

## Context
ADR-0054 builds the mechanism for optional modules and the administrator CLI, and stops there on
purpose. Several things were discussed that are not built, and each of them would be easy to start
by accident because the mechanism makes it look close. They are recorded here so that nobody
mistakes them for decided, and so that the next person who wants one knows what it needs.

## Decision
Nothing below is built, scaffolded or reserved in the code, except the name in item 3.

1. **Widgets** (a piece of UI an institution places on a page) are not designed. The module web UI of
   ADR-0054 is a whole page at `/m/<name>/...`, not a widget.
2. **Plugins in the sense of loading code at run time, and installing third-party plugins, are out of
   scope.** A module is compiled in by a Cargo feature and is code of this repository or of a fork.
   There is no dynamic loading, no plugin directory, no package registry, no sandbox for foreign code,
   and no signature check. Anything that runs foreign code needs its own ADR on the trust model first.
3. **An end-user CLI** (students and teachers, a normal API client with a token in the OS keychain,
   as ADR-0014 sketched with `keyring`) is not built. **The name `vumlo-cli` belongs to the
   administrator tool and is not available to it.** The end-user CLI gets a different name when it is
   built, and ARCHITECTURE.md says so.
4. **Native clients** (desktop, mobile): no config keys, no public/private marks and no endpoints are
   added for clients that do not exist. When one does, its needs are added to the schema with their
   visibility.
5. **An authenticated route for a module.** A module router receives a `ModuleContext` (the pool and
   its own settings) and nothing of `Actor` or `policy`, because both live in `server` and a module
   must not depend on it. The test-only `dummy` module's routes are unauthenticated. The first real
   module with routes needs an extractor and a policy hook in `module-api`; that is an ADR of its own.
6. **The modules themselves** (`ai-agent`, `vpl`) are not in the registry. Listing them would tell an
   operator to build with a feature that does not exist.
7. **Release binaries on GitHub Releases** are not produced. Installations build from the repository
   or use the Docker images they build themselves (`docs/install.md`).
8. **Rolling back a migration.** `update.sh` restores from the backup; there are no down migrations.

## Consequences
- Anyone who proposes one of these starts from this list and writes the missing decision, instead of
  finding a half-built version.
- The `dummy` module is the only module in the tree; it must keep being excluded from default builds
  and from the documentation for operators.
