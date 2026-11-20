#!/bin/sh
# Cargo runner: a test binary gets its own TMPDIR, removed when it exits, so
# the temp dirs tests make never pile up. Examples and bins run as they are.
case "$1" in
*/deps/*)
    base="${TMPDIR:-/tmp}"
    # Sweep what a killed run left behind.
    find "$base" -maxdepth 1 -name 'zgx-testrun.*' -mmin +120 -exec rm -rf {} + 2>/dev/null
    run=$(mktemp -d "${base%/}/zgx-testrun.XXXXXX") || exec "$@"
    trap 'rm -rf "$run"' EXIT
    trap 'exit 130' INT TERM HUP
    TMPDIR="$run" "$@"
    status=$?
    exit $status
    ;;
*) exec "$@" ;;
esac
