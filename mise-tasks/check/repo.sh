#!/usr/bin/env bash
#MISE description="Shared repository checks, then the repository's own check:local"
#MISE depends=["format:toml", "lint:md", "lint:typos", "lint:workflows", "specs", "check:local"]
# Repository checks every project type runs. verify calls check:repo: the
# shared checks, then check:local, which check/local.sh leaves to the
# repository. The tools come from mise.toml.
