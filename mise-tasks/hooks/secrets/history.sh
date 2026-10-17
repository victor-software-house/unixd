#!/usr/bin/env bash
#MISE description="Scan the full history for credentials"
# gitleaks comes from mise.dev.toml.
gitleaks git --redact --no-banner
