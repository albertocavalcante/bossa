#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$BOSSA_TEST_TRACE"
case "$1" in
export)
	if [ -f "$BOSSA_TEST_DOCK_STATE" ]; then
		stage=$(/bin/cat "$BOSSA_TEST_DOCK_STATE")
	else
		stage=0
	fi
	/bin/cat "$BOSSA_TEST_DOCK_SNAPSHOTS/$stage"
	;;
--move)
	case "$2" in
	A) printf '1\n' >"$BOSSA_TEST_DOCK_STATE" ;;
	B) printf '2\n' >"$BOSSA_TEST_DOCK_STATE" ;;
	*) exit 1 ;;
	esac
	;;
Dock) : ;;
*) exit 1 ;;
esac
