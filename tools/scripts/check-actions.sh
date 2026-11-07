#!/bin/sh
set -eu

# Keep the same validator versions in local hooks and CI.
case "$(pinact --version)" in
"pinact version 5.0.2" | "pinact version v5.0.2") ;;
*)
	echo "Expected pinact 5.0.2; install github.com/suzuki-shunsuke/pinact/v5/cmd/pinact@v5.0.2" >&2
	exit 1
	;;
esac

# Use the same authenticated identity as gh when the caller has not supplied a
# pinact token. Command substitution stays inside this process; never print it.
if [ -z "${PINACT_GITHUB_TOKEN:-}${GITHUB_TOKEN:-}" ]; then
	PINACT_GITHUB_TOKEN=$(gh auth token)
	export PINACT_GITHUB_TOKEN
fi

ghalint run -c tools/lint/ghalint.yaml
pinact run --fix=false --verify-comment -c tools/lint/pinact.yaml

# These exact commits have no version tag. pinact's matching exceptions are
# backed by upstream ancestry checks, never by an unchecked repository ignore.
# Compare FROM the pin TO the trusted branch: a fork-only descendant is behind
# or diverged, and must not pass. API failures also stop validation.
verify_untagged() {
	action_repo=$1
	action_commit=$2
	upstream_branch=$3
	comparison=$(gh api "repos/$action_repo/compare/$action_commit...$upstream_branch" --jq .status)
	case "$comparison" in
	ahead | identical) ;;
	*)
		echo "Unverified action pin: $action_repo@$action_commit ($comparison)" >&2
		return 1
		;;
	esac
}

verify_untagged dtolnay/rust-toolchain 888c2e1ea69ab0d4330cbf0af1ecc7b68f368cc1 stable
verify_untagged albertocavalcante/actions 9a91e672159e2d4d806bce6fca6199ec65852947 main
