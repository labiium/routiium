#!/usr/bin/env bash
set -euo pipefail

# Security audit gate for Routiium.
#
# The ignored advisories below are currently transitive through optional or
# upstream-pinned dependency stacks that cannot be upgraded independently in this
# crate without removing the feature entirely:
# - rustls-webpki 0.101/0.102 via AWS Smithy legacy rustls and optional libsql.
# - bincode/rustls-pemfile via optional libsql/turso.
# - fxhash/instant via sled's storage stack.
#
# Do not add a new ignore here without recording the dependency path and removal
# plan in the PR/commit. New, non-ignored vulnerabilities still fail CI.
exec cargo audit --deny warnings \
  --ignore RUSTSEC-2026-0098 \
  --ignore RUSTSEC-2026-0099 \
  --ignore RUSTSEC-2026-0104 \
  --ignore RUSTSEC-2026-0049 \
  --ignore RUSTSEC-2025-0141 \
  --ignore RUSTSEC-2025-0057 \
  --ignore RUSTSEC-2024-0384 \
  --ignore RUSTSEC-2025-0134
