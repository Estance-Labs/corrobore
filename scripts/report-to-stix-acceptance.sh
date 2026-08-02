#!/usr/bin/env sh
# Copyright (c) 2026 AreDee-Bangs
# SPDX-License-Identifier: MIT
set -eu

# The GitHub job supplies the outer timeout; HTTP operations also use the
# server's bounded request timeout. Keep this command focused and reproducible.
cargo test \
  -p corrobore-http-server \
  --test report_to_stix_acceptance \
  --no-default-features \
  --features enterprise-cti \
  --locked
