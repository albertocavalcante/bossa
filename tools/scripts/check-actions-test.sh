#!/usr/bin/env bash
set -euo pipefail

export GITHUB_TOKEN=fixture-token
fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT

cat >"$fixture_dir/pinact" <<'STUB'
#!/bin/sh
set -eu
if [ "$1" = --version ]; then
  echo "pinact version ${PINACT_VERSION:-5.0.2}"
else
  exit "${PINACT_RESULT:-0}"
fi
STUB
cat >"$fixture_dir/ghalint" <<'STUB'
#!/bin/sh
exit 0
STUB
cat >"$fixture_dir/gh" <<'STUB'
#!/bin/sh
set -eu
if [ "$1 $2" = "auth token" ]; then
  if [ "${GH_AUTH_RESULT:-0}" != 0 ]; then exit 1; fi
  echo fixture-token
  exit 0
fi
case "$2" in
  repos/dtolnay/rust-toolchain/compare/888c2e1ea69ab0d4330cbf0af1ecc7b68f368cc1...stable | repos/albertocavalcante/actions/compare/9a91e672159e2d4d806bce6fca6199ec65852947...main) ;;
  *) exit 1 ;;
esac
if [ "$GH_RESULT" = api-error ]; then
  exit 1
fi
echo "$GH_RESULT"
STUB
chmod +x "$fixture_dir/pinact" "$fixture_dir/ghalint" "$fixture_dir/gh"

for comparison in ahead identical; do
	PATH="$fixture_dir:$PATH" GH_RESULT="$comparison" sh tools/scripts/check-actions.sh
done

PATH="$fixture_dir:$PATH" GH_RESULT=ahead PINACT_VERSION=v5.0.2 sh tools/scripts/check-actions.sh
env -u GITHUB_TOKEN -u PINACT_GITHUB_TOKEN PATH="$fixture_dir:$PATH" GH_RESULT=ahead sh tools/scripts/check-actions.sh
if env -u GITHUB_TOKEN -u PINACT_GITHUB_TOKEN PATH="$fixture_dir:$PATH" GH_RESULT=ahead GH_AUTH_RESULT=1 sh tools/scripts/check-actions.sh >"$fixture_dir/output" 2>&1; then
	echo "Ignored gh authentication failure" >&2
	exit 1
fi
if PATH="$fixture_dir:$PATH" GH_RESULT=ahead PINACT_VERSION=5.0.1 sh tools/scripts/check-actions.sh >"$fixture_dir/output" 2>&1; then
	echo "Accepted an unexpected validator version" >&2
	exit 1
fi

for comparison in behind diverged unexpected api-error; do
	if PATH="$fixture_dir:$PATH" GH_RESULT="$comparison" sh tools/scripts/check-actions.sh >"$fixture_dir/output" 2>&1; then
		echo "Accepted invalid ancestry result: $comparison" >&2
		exit 1
	fi
done

if PATH="$fixture_dir:$PATH" GH_RESULT=ahead PINACT_RESULT=1 sh tools/scripts/check-actions.sh; then
	echo "Ignored pinact verification failure" >&2
	exit 1
fi

# A changed SHA must stop matching the documented untagged exception.
echo 'version: 3' >"$fixture_dir/global-pinact.yaml"
printf '%s\n' 'uses: dtolnay/rust-toolchain@888c2e1ea69ab0d4330cbf0af1ecc7b68f368cc1 # untagged commit' >"$fixture_dir/action.yml"
PINACT_GLOBAL_CONFIG="$fixture_dir/global-pinact.yaml" pinact run --fix=false --no-api -c tools/lint/pinact.yaml "$fixture_dir/action.yml"
printf '%s\n' 'uses: dtolnay/rust-toolchain@0000000000000000000000000000000000000000 # untagged commit' >"$fixture_dir/action.yml"
if PINACT_GLOBAL_CONFIG="$fixture_dir/global-pinact.yaml" pinact run --fix=false --no-api -c tools/lint/pinact.yaml "$fixture_dir/action.yml" >"$fixture_dir/output" 2>&1; then
	echo "Changed SHA matched an untagged exception" >&2
	exit 1
fi

echo "Action validator failure cases passed"
