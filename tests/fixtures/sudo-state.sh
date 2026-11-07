#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$BOSSA_TEST_SUDO_TRACE"
if [ "${BOSSA_TEST_SUDO_GATE:-}" = "$1" ]; then
	: >"$BOSSA_TEST_SUDO_READY"
	exec /bin/sleep 3
fi
case "$1" in
-v) exit "${BOSSA_TEST_SUDO_ACQUIRE_STATUS:-0}" ;;
-k) exit "${BOSSA_TEST_SUDO_CLEANUP_STATUS:-0}" ;;
-n) exit 0 ;;
*) exit 1 ;;
esac
