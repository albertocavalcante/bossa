#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$BOSSA_TEST_TRACE"
if [ -n "${BOSSA_TEST_ENV_TRACE:-}" ]; then
	printf '%s|%s|%s|%s|%s|%s\n' "$*" "${LC_ALL:-}" "${COREPACK_ENABLE_PROJECT_SPEC:-}" "${COREPACK_ENABLE_AUTO_PIN:-}" "${COREPACK_ENABLE_NETWORK:-}" "$PWD" >>"$BOSSA_TEST_ENV_TRACE"
fi
if [ "$1" = '--version' ]; then
	printf 'Homebrew fixture\n'
else
	/bin/cat "$BOSSA_TEST_RESPONSE"
fi
