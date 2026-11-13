#!/bin/sh
set -eu
/usr/bin/head -c "$1" /dev/zero
/usr/bin/head -c "$2" /dev/zero >&2
