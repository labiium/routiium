#!/usr/bin/env bash
set -euo pipefail

# Security audit gate for Routiium. Never ignore vulnerability advisories.
# These two existing maintenance-only exceptions remain in sled 0.34's stack:
# - fxhash: sled -> fxhash 0.2 (no maintained drop-in release).
# - instant: sled -> parking_lot 0.11 -> instant 0.1.
# Removal plan: migrate the sled-backed key/history stores to a maintained
# backend, with on-disk data migration tests, before removing these exceptions.
# Do not add an ignore without documenting its dependency path and removal plan.
exec cargo audit --deny warnings \
  --ignore RUSTSEC-2025-0057 \
  --ignore RUSTSEC-2024-0384
