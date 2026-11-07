#!/bin/sh
set -eu
case "$1" in
export)
	printf '%s' '<plist version="1.0"><dict><key>Enabled</key>'
	if [ -f "$BOSSA_TEST_DEFAULT_STATE" ]; then
		printf '%s' '<true/>'
	else
		printf '%s' '<false/>'
	fi
	printf '%s\n' '</dict></plist>'
	;;
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
