#!/usr/bin/env bash

set -euo pipefail

# Host-side operator command. It will validate the versioned dataset
# selection before any Docker call, address the same Compose project as the
# running distribution, prove OpenCTI health, and invoke the in-container
# loader without putting the administrator token in host process arguments.

DEFAULT_DATASETS="corrobore-demo"
DATASETS="${1:-${OPENCTI_DEMO_DATASETS:-${DEFAULT_DATASETS}}}"
COMPOSE_FILE="${OPENCTI_CORROBORE_COMPOSE_FILE:-packaging/opencti-elastic-free/compose.yml}"
COMPOSE_ENV_FILE="${OPENCTI_CORROBORE_ENV_FILE:-}"
PROJECT_NAME="${OPENCTI_CORROBORE_PROJECT_NAME:-}"

fail() {
  printf 'demo data error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

validate_datasets() {
  [[ $# -le 1 ]] || fail "usage: $0 [corrobore-demo]"
  [[ "${DATASETS}" == "corrobore-demo" ]] ||
    fail "unsupported demo dataset selection: ${DATASETS}"
}

compose() {
  local arguments=()
  if [[ -n "${PROJECT_NAME}" ]]; then
    arguments+=(--project-name "${PROJECT_NAME}")
  fi
  if [[ -n "${COMPOSE_ENV_FILE}" ]]; then
    arguments+=(--env-file "${COMPOSE_ENV_FILE}")
  fi
  arguments+=(-f "${COMPOSE_FILE}")
  docker compose "${arguments[@]}" "$@"
}

validate_datasets "$@"
require_command docker

compose exec -T opencti node -e \
  "const k=require('node:fs').readFileSync('/run/secrets/opencti-health-key','utf8').trim();fetch('http://127.0.0.1:8080/health?health_access_key='+encodeURIComponent(k)).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))" ||
  fail "the OpenCTI service is not healthy"

printf 'Loading versioned OpenCTI demonstration dataset: %s\n' "${DATASETS}"
compose exec -T opencti /usr/local/bin/opencti-load-demo-data "${DATASETS}"
