#!/usr/bin/env bash
# Updates an installation to the release that is on disk: back up, stop, migrate, start, check.
#
#   deploy/update.sh --systemd     binaries in /opt/vumlo/bin, units vumlo-core, vumlo-worker, vumlo-web
#   deploy/update.sh --docker      from the repository root, with deploy/compose.yaml
#
# Get the new release first (git pull, then build and install the binaries; or for Docker the
# script rebuilds the images itself). The order is fixed and never changes:
#
#   1. the backup, written and checked      <- nothing is touched before it exists and reads back
#   2. stop the services
#   3. migrate the database
#   4. start the services
#   5. doctor
#
# Any failing step stops the script there (set -e). A migration is never run without the backup.
# If it stops after step 2 the services stay stopped: nothing runs against a half-migrated
# database. Restore with the backup (docs/install.md, "Backups") or fix the cause and run it again.

set -euo pipefail

mode="${1:-}"
backups="${VUMLO_BACKUP_DIR:-./backups}"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
dump="$backups/vumlo-$stamp.dump"
step="starting"

on_error() {
    echo >&2
    echo "update.sh: FAILED during: $step" >&2
    if [ -s "$dump" ]; then
        echo "update.sh: the backup is $dump" >&2
    fi
    echo "update.sh: nothing after the failed step was run. The services may be stopped." >&2
}
trap on_error ERR

case "$mode" in
    --systemd)
        cli="${VUMLO_CLI:-/opt/vumlo/bin/vumlo-cli}"
        config="${VUMLO_CONFIG:-/etc/vumlo/vumlo.toml}"
        envfile="${VUMLO_ENV_FILE:-/etc/vumlo/vumlo.env}"
        units="vumlo-core vumlo-worker vumlo-web"
        # DATABASE_URL and the other secrets, for pg_dump and for the CLI.
        set -a
        # shellcheck disable=SC1090
        . "$envfile"
        set +a
        export VUMLO_CONFIG="$config"

        backup() {
            # libpq takes a connection string from PGDATABASE: the password stays out of `ps`.
            PGDATABASE="$DATABASE_URL" pg_dump --format=custom --file "$dump"
            pg_restore --list "$dump" >/dev/null
        }
        stop() { sudo systemctl stop $units; }
        migrate() { "$cli" db migrate; }
        start() { sudo systemctl start $units; }
        check() { "$cli" doctor; }
        prepare() { "$cli" version; "$cli" config check; }
        ;;
    --docker)
        cd "$(dirname "$0")/.."
        export COMPOSE_FILE=deploy/compose.yaml COMPOSE_ENV_FILES=.env
        VUMLO_UID="$(id -u)"
        VUMLO_GID="$(id -g)"
        export VUMLO_UID VUMLO_GID

        backup() {
            docker compose up -d --wait postgres
            docker compose exec -T postgres pg_dump -U vumlo --format=custom vumlo >"$dump"
            docker compose exec -T postgres pg_restore --list <"$dump" >/dev/null
        }
        stop() { docker compose stop web worker core; }
        migrate() { docker compose run --rm vumlo-cli db migrate; }
        start() { docker compose up -d; }
        check() { docker compose run --rm vumlo-cli doctor; }
        prepare() {
            # The images are the release: build them before anything is stopped.
            docker compose run --rm --no-deps vumlo-cli config check
            docker compose run --rm --no-deps vumlo-cli brand apply
            docker compose build
        }
        ;;
    *)
        echo "usage: $0 --systemd | --docker" >&2
        exit 2
        ;;
esac

mkdir -p "$backups"
chmod 700 "$backups"

step="checking the configuration and preparing the release (nothing is stopped yet)"
prepare

step="the backup (nothing was changed)"
backup
[ -s "$dump" ] || { echo "update.sh: the backup is empty" >&2; exit 1; }
echo "backup: $dump ($(wc -c <"$dump") bytes, readable by pg_restore)"

step="stopping the services"
stop

step="migrating the database (the services are stopped; the backup is $dump)"
migrate

step="starting the services"
start

step="doctor"
check

trap - ERR
echo "update.sh: done"
