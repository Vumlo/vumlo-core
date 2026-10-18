#!/usr/bin/env bash
# Builds vumlo-<version>-source.tar.gz and SHA256SUMS in <output-dir> from the tracked files of HEAD.
#
#   .github/release/make-archive.sh 0.1.0 dist        (from the repository root)
#
# What is in the archive is the allowlist below, nothing else: what an administrator needs to
# build and run Vumlo. Not in it: tests, ADRs, CI files, AGENTS.md, the development compose file,
# and the unfinished or test-only modules. Removing the test-only module `dummy` changes the
# manifests, so the archive carries generated ones (see "release manifests"); the result is
# built from scratch by verify-archive.sh before anything is published.
set -euo pipefail

version="${1:?usage: make-archive.sh <version> <output-dir>}"
out="$(mkdir -p "${2:?usage: make-archive.sh <version> <output-dir>}" && cd "$2" && pwd)"
name="vumlo-$version"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/$name"

# --- the allowlist ------------------------------------------------------------------------------
allow=(
    backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml
    backend/migrations backend/.sqlx backend/crates
    web deploy brand
    vumlo .dockerignore
    docs/install.md CHANGELOG.md README.md
)
# A licence file, once the project has one.
while IFS= read -r licence; do allow+=("$licence"); done < <(git ls-files 'LICENSE*' 'COPYING*')

git archive --format=tar "--prefix=$name/" HEAD -- "${allow[@]}" | tar -x -C "$work"

# --- what is left out of the directories above ----------------------------------------------------
rm -rf \
    "$stage"/backend/crates/*/tests \
    "$stage"/backend/crates/modules \
    "$stage"/web/e2e "$stage"/web/playwright.config.ts "$stage"/web/src/modules/dummy \
    "$stage"/web/AGENTS.md "$stage"/web/API_GAPS.md "$stage"/web/pnpm-lock.yaml

# --- release manifests ----------------------------------------------------------------------------
# The test-only module `dummy` is a workspace member, an optional dependency and a feature of
# `server` and `vumlo-cli`, and a line of each module list. None of that ships.
sed -i -e '/crates\/modules/d' -e '/dummy/d' "$stage/backend/Cargo.toml"
sed -i -e '/dummy/d' "$stage/backend/crates/server/Cargo.toml" "$stage/backend/crates/vumlo-cli/Cargo.toml" \
    "$stage/backend/crates/config/Cargo.toml"
sed -i -e '/feature = "dummy"/d' -e '/vumlo_module_dummy/d' \
    "$stage/backend/crates/server/src/modules.rs" "$stage/backend/crates/vumlo-cli/src/runtime.rs"
# The offline query data of the module's own queries.
{ grep -rl 'dummy\.' "$stage/backend/.sqlx" || true; } | xargs -r rm -f

# Cargo.lock: let cargo drop the module's entries and nothing else. Any other change is a mistake.
(cd "$stage/backend" && cargo metadata --format-version 1 >/dev/null)
if diff <(git show HEAD:backend/Cargo.lock) "$stage/backend/Cargo.lock" | grep -q '^>'; then
    echo "make-archive: Cargo.lock gained lines; only removals are expected" >&2
    exit 1
fi

# The configuration to copy: the same text `vumlo-cli init` writes.
cp "$stage/deploy/vumlo.example.toml" "$stage/vumlo.toml.example"

# Nothing of the excluded kind may be left.
if find "$stage" \( -path '*/tests' -o -path '*/e2e' -o -path '*/decisions' -o -path '*/.github' \) -print -quit | grep -q .; then
    echo "make-archive: a test, ADR or CI directory is in the archive" >&2
    exit 1
fi
if grep -rIl 'vumlo-module-dummy\|vumlo_module_dummy' "$stage" | grep -q .; then
    echo "make-archive: a reference to the unfinished module is in the archive" >&2
    exit 1
fi

# --- the archive ----------------------------------------------------------------------------------
archive="$name-source.tar.gz"
( cd "$work" && tar --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@$(git -C "$OLDPWD" log -1 --format=%ct)" -cf - "$name" ) | gzip -n -9 >"$out/$archive"
( cd "$out" && sha256sum "$archive" >SHA256SUMS )
echo "make-archive: $out/$archive"
cat "$out/SHA256SUMS"
