#!/bin/sh
set -eu
case "$1" in
read)
	if [ -f "$BOSSA_TEST_DEFAULT_STATE" ]; then
		printf '1\n'
	else
		printf '0\n'
	fi
	;;
write)
	: >"$BOSSA_TEST_DEFAULT_STATE"
	;;
*)
	exit 1
	;;
esac
