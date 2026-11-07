#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$BOSSA_TEST_TRACE"
case "$1" in
--version) printf 'Homebrew fixture\n' ;;
--prefix) printf '%s\n' "$BOSSA_TEST_BREW_ROOT" ;;
--cellar) printf '%s/Cellar\n' "$BOSSA_TEST_BREW_ROOT" ;;
--caskroom) printf '%s/Caskroom\n' "$BOSSA_TEST_BREW_ROOT" ;;
--repository) printf '%s/repo\n' "$BOSSA_TEST_BREW_ROOT" ;;
*) exit 0 ;;
esac
