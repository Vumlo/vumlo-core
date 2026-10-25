#!/usr/bin/env bash
# Proves that a release archive works, by following docs/install.md (the Docker path) in a clean
# directory: verify the checksum, unpack, init, configure, replace the placeholder logo, config
# check, brand apply, build, migrate, create an administrator, start, doctor, and ask the API and
# the web app. Everything it creates is removed at the end.
#
#   .github/release/verify-archive.sh <directory with the archive and SHA256SUMS> <version>
#
# The administrator's password is generated and kept in a variable only. It is printed once when
# VERIFY_PRINT_PASSWORD=1 (a throwaway installation, on this machine), never otherwise.
set -euo pipefail

from="$(cd "${1:?usage: verify-archive.sh <dir> <version>}" && pwd)"
version="${2:?usage: verify-archive.sh <dir> <version>}"
work="$(mktemp -d)"
export COMPOSE_FILE=deploy/compose.yaml COMPOSE_ENV_FILES=.env

teardown() {
    status=$?
    if [ -d "$work/vumlo-$version" ]; then
        (cd "$work/vumlo-$version" && docker compose down --volumes --rmi local --remove-orphans >/dev/null 2>&1) || true
    fi
    rm -rf "$work"
    exit "$status"
}
trap teardown EXIT

step() { echo; echo "== $*"; }

cd "$work"
cp "$from/vumlo-$version-source.tar.gz" "$from/SHA256SUMS" .

step "1. verify and unpack"
sha256sum --check --ignore-missing SHA256SUMS
tar xzf "vumlo-$version-source.tar.gz"
cd "vumlo-$version"
for unwanted in .github docs/decisions backend/crates/server/tests backend/crates/modules backend/crates/test-support ARCHITECTURE.md AGENTS.md; do
    [ ! -e "$unwanted" ] || { echo "the archive contains $unwanted" >&2; exit 1; }
done
# No crate ships its tests or the test harness: every `tests.rs` is deleted, and a `mod tests;` declaration
# behind #[cfg(test)] compiles away.
leftover="$(find backend/crates -type f \( -name tests.rs -o -name content_tests.rs -o -name test_support.rs \) -print)"
[ -z "$leftover" ] || { echo "the archive contains test files:" >&2; echo "$leftover" >&2; exit 1; }
leftover="$(find backend/crates -type d \( -name tests -o -name test-support \) -print)"
[ -z "$leftover" ] || { echo "the archive contains test directories:" >&2; echo "$leftover" >&2; exit 1; }
for wanted in vumlo vumlo.toml.example docs/install.md docs/desktop.md CHANGELOG.md brand/logo.png deploy/compose.yaml backend/.sqlx desktop/scripts/build.mjs desktop/release/desktop-release.yml desktop/Cargo.lock; do
    [ -e "$wanted" ] || { echo "the archive lacks $wanted" >&2; exit 1; }
done

node --check desktop/scripts/build.mjs
node --check desktop/scripts/config.mjs

step "2. configure (init, .env, vumlo.toml, logo)"
./vumlo init
cp deploy/env.example .env && chmod 600 .env
db_password="$(openssl rand -hex 16)"
sed -i "s/change-me/$db_password/g" .env
grep -q '^name = "Vumlo"' vumlo.toml          # the placeholder name
printf '%s\n' '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#b00020"/></svg>' >brand/own-logo.svg
sed -i 's#^logo = "brand/logo.png"#logo = "brand/own-logo.svg"#' vumlo.toml
sed -i 's/^name = "Vumlo"/name = "Verification University"/' vumlo.toml

step "3. config check, brand apply"
./vumlo config check
./vumlo brand apply
grep -q 'Verification University' .vumlo/generated/client-config.json
[ ! -e .vumlo/generated/logo.png ]   # the placeholder is not what was generated
[ "$(cat .vumlo/generated/logo.svg)" = "$(cat brand/own-logo.svg)" ]

step "4. build"
./vumlo version
docker compose build

step "5. migrate, administrator, start"
docker compose up -d --wait postgres
./vumlo db migrate
admin_password="$(openssl rand -base64 24 | tr -d '/+=' | cut -c1-24)"
printf '%s\n' "$admin_password" | ./vumlo admin create admin@verify.example --name Ver --surname Ify --password-stdin
if [ "${VERIFY_PRINT_PASSWORD:-0}" = 1 ]; then echo "throwaway administrator: admin@verify.example / $admin_password"; fi
docker compose up -d

step "6. doctor, the API, the web app"
ok=0
for _ in $(seq 1 60); do
    if curl -fsS http://127.0.0.1:8080/api/v1/health >/dev/null 2>&1 && curl -fsS http://127.0.0.1:3000/ >/dev/null 2>&1; then ok=1; break; fi
    sleep 2
done
[ "$ok" = 1 ] || { docker compose ps; docker compose logs --tail 40 core web; echo "the services did not come up" >&2; exit 1; }
./vumlo doctor
curl -fsS http://127.0.0.1:8080/api/v1/institution | tee /dev/stderr | grep -q 'Verification University'
curl -fsS http://127.0.0.1:3000/api/v1/institution | grep -q 'Verification University'   # through the web app's proxy
curl -fsS http://127.0.0.1:3000/ | grep -q 'Verification University'
login="$(printf '{"email":"admin@verify.example","password":"%s","client":"app"}' "$admin_password")"
curl -fsS -H 'content-type: application/json' -d "$login" http://127.0.0.1:8080/api/v1/auth/login >/dev/null
for icon in favicon.ico icon-192.png site.webmanifest; do
    curl -fsS -o /dev/null "http://127.0.0.1:3000/brand/$icon"
done
echo
echo "verify-archive: vumlo-$version-source.tar.gz works"
