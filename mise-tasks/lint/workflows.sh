#!/usr/bin/env bash
#MISE description="GitHub Actions workflow lint"
# actionlint lints the workflows of the git repository it runs in, so the
# check looks where actionlint does: the repository root, or this directory
# outside a repository. actionlint fails where there is no workflow, such as
# a fresh Bun render, so that case passes with a notice.
shopt -s nullglob
root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
workflows=("$root"/.github/workflows/*.yml "$root"/.github/workflows/*.yaml)
if [ "${#workflows[@]}" -eq 0 ]; then
  echo "no workflow in $root/.github/workflows, nothing to lint"
  exit 0
fi
actionlint
