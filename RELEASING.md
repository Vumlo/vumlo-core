# Releasing

Release only when CI is green on the commit you tag and `CHANGELOG.md` has its entry. Before 1.0 a
**minor** version (`0.2.0`) is for anything that changes the configuration (`config_version`), the API
or the migrations; a **patch** version (`0.1.1`) is for fixes that need none of those. Whoever
installs a release reads its **Upgrade notes** in the changelog first.

1. Branch from `main`. Set the new version in all three places (a test fails if they differ):
   `[workspace.package] version` in `backend/Cargo.toml`, `"version"` in `web/package.json`, and the
   default of `VUMLO_VERSION` in `deploy/compose.yaml`. Run `cargo update --workspace` in `backend/`
   so `Cargo.lock` follows.
2. In `CHANGELOG.md` move the `[Unreleased]` entries under `## [X.Y.Z] - YYYY-MM-DD`, write the
   **Upgrade notes** (a new `config_version`, a manual step, the migrations that run), and update the
   links at the bottom.
3. Run the checks of `AGENTS.md` §14. Merge the branch once CI is green.
4. Tag the merge commit and push the tag: `git tag -a vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z`.
5. The `Release` workflow checks that the tag is the workspace version, runs the CI checks again,
   pushes `ghcr.io/vumlo/vumlo-backend:X.Y.Z` and `ghcr.io/vumlo/vumlo-web:X.Y.Z`, and creates the
   GitHub Release with the changelog entry as its notes. Check the Release page and the two packages.

A published tag is never moved or deleted. If a release is wrong, fix it on `main` and release the
next patch version.
