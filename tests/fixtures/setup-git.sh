#!/bin/sh
set -eu

previous=''
operation=''
last=''
destination=''
for argument; do
	if [ "${BOSSA_TEST_GIT_FAILURE-}" = unsupported-observation ] && [ "$argument" = --no-lazy-fetch ]; then
		exit 129
	fi
	if [ "$previous" = worktree ] && [ "$argument" = add ]; then
		operation=worktree
	fi
	if [ "$argument" = clone ]; then
		operation=clone
	fi
	destination=$last
	last=$argument
	previous=$argument
done

case "${BOSSA_TEST_GIT_FAILURE-}:$operation:$last" in
clone:clone:*)
	/bin/mkdir -p "$last"
	printf 'partial clone retained\n' >"$last/partial"
	exit 23
	;;
no-repository:clone:*)
	exit 0
	;;
worktree:worktree:dev)
	/bin/mkdir -p "$destination"
	printf 'partial worktree retained\n' >"$destination/partial"
	exit 23
	;;
esac

exec /usr/bin/git "$@"
