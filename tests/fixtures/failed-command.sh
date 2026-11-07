#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$BOSSA_TEST_TRACE"
exit 1
