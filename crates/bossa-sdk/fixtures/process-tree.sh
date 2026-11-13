#!/bin/sh
set -eu
printf '%s\n' "$$" >"$1"
/bin/sleep 3 &
printf '%s\n' "$!" >>"$1"
if [ "${2:-wait}" = leave ]; then
	exit 0
fi
wait
