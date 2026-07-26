#!/usr/bin/env bash

set -euo pipefail

# Durable, monotonic migration state machine. Every phase is idempotent and the
# lock prevents two operators from advancing routing or write authority at once.
PHASE="${1:-}"
STATE_DIR="${CORROBORE_MIGRATION_STATE_DIR:-packaging/opencti-elastic-free/state}"
STATE_FILE="${STATE_DIR}/migration.json"
POLICY_FILE="${STATE_DIR}/read-routing.json"
LOCK_FILE="${STATE_DIR}/migration.lock"
COMPOSE_FILE="${OPENCTI_CORROBORE_COMPOSE_FILE:-packaging/opencti-elastic-free/compose.yml}"
COMPOSE_MIGRATION_FILE="${OPENCTI_CORROBORE_MIGRATION_COMPOSE_FILE:-packaging/opencti-elastic-free/compose.migration.yml}"
MIGRATING_REFERENCE="${OPENCTI_MIGRATION_FROM_REFERENCE:-false}"
CORROBORE_URL="${CORROBORE_URL:-https://127.0.0.1:8080}"
TOKEN_FILE="${CORROBORE_AUTH_TOKEN_FILE:-packaging/opencti-elastic-free/secrets/corrobore-http-token}"
CA_FILE="${CORROBORE_CA_FILE:-packaging/opencti-elastic-free/secrets/tls.crt}"
SAFETY_DELAY="${CORROBORE_MIGRATION_SAFETY_DELAY_SECONDS:-86400}"

ORDER=(install initial-import catch-up validate shadow canary primary-read primary-write safety-delay shutdown-elastic)

usage() {
  printf 'usage: %s <%s|rollback|status>\n' "$0" "$(IFS='|'; echo "${ORDER[*]}")" >&2
}

fail() {
  printf 'migration error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

compose() {
  if [[ "${MIGRATING_REFERENCE}" == "true" ]]; then
    docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_MIGRATION_FILE}" "$@"
  else
    docker compose -f "${COMPOSE_FILE}" "$@"
  fi
}

require_reference_migration() {
  [[ "${MIGRATING_REFERENCE}" == "true" ]] ||
    fail "${PHASE} is only valid with OPENCTI_MIGRATION_FROM_REFERENCE=true"
}

phase_index() {
  local candidate=$1
  local index
  for index in "${!ORDER[@]}"; do
    [[ "${ORDER[$index]}" == "${candidate}" ]] && { echo "${index}"; return; }
  done
  return 1
}

initialize_state() {
  mkdir -p "${STATE_DIR}"
  if [[ ! -f "${STATE_FILE}" ]]; then
    local temporary="${STATE_FILE}.tmp.$$"
    jq -n '{schema_version:1,current_phase:null,history:[],safety_delay_started_at:null,safety_delay_started_epoch:null}' >"${temporary}"
    chmod 0600 "${temporary}"
    mv "${temporary}" "${STATE_FILE}"
  fi
}

record_phase() {
  local phase=$1
  local now temporary
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  temporary="${STATE_FILE}.tmp.$$"
  jq --arg phase "${phase}" --arg now "${now}" --argjson epoch "$(date +%s)" \
    '.current_phase=$phase | .history += [{phase:$phase,completed_at:$now}] |
     if $phase == "safety-delay" then .safety_delay_started_at=$now | .safety_delay_started_epoch=$epoch else . end' \
    "${STATE_FILE}" >"${temporary}"
  chmod 0600 "${temporary}"
  mv "${temporary}" "${STATE_FILE}"
}

assert_next_phase() {
  local requested=$1 current requested_index current_index
  requested_index="$(phase_index "${requested}")" || fail "unknown phase: ${requested}"
  current="$(jq -r '.current_phase // empty' "${STATE_FILE}")"
  if [[ -z "${current}" ]]; then
    [[ "${requested_index}" -eq 0 ]] || fail "install must be completed first"
    return
  fi
  current_index="$(phase_index "${current}")"
  if [[ "${requested_index}" -eq "${current_index}" ]]; then
    return
  fi
  [[ "${requested_index}" -eq $((current_index + 1)) ]] ||
    fail "cannot move from ${current} to ${requested}; run ${ORDER[$((current_index + 1))]}"
}

token() {
  [[ -s "${TOKEN_FILE}" ]] || fail "missing Corrobore token file: ${TOKEN_FILE}"
  tr -d '\r\n' <"${TOKEN_FILE}"
}

api() {
  local method=$1 path=$2 body=${3:-}
  local args=(--fail --silent --show-error --max-time 60 --request "${method}" --cacert "${CA_FILE}" --header "Authorization: Bearer $(token)")
  if [[ -n "${body}" ]]; then
    args+=(--header "Content-Type: application/json" --data-binary "@${body}")
  fi
  curl "${args[@]}" "${CORROBORE_URL}${path}"
}

write_policy() {
  local mode=$1 percentage=$2
  local temporary="${POLICY_FILE}.tmp.$$"
  jq -n --arg mode "${mode}" --arg version "migration-${mode}-$(date -u +%Y%m%d%H%M%S)" --argjson percentage "${percentage}" '{
    policy_version:$version,
    mode:$mode,
    default_percentage_basis_points:$percentage,
    rules:[],
    thresholds:{max_error_rate_basis_points:100,max_latency_p95_ms:120,minimum_soak_requests:10000}
  }' >"${temporary}"
  chmod 0600 "${temporary}"
  mv "${temporary}" "${POLICY_FILE}"
}

restart_corrobore() {
  compose up -d --no-deps --wait corrobore
}

assert_parity_gate() {
  local sync writes
  sync="$(api GET /v1/opencti/sync/status)"
  writes="$(api GET /v1/opencti/writes/status)"
  jq -e '.ok == true and .result.lag == 0 and .result.queue_depth == 0 and
    .result.rejected_operations == 0 and .result.quarantined_operations == 0 and
    .result.shadow_reads_enabled == true and .result.divergence == "InSync"' <<<"${sync}" >/dev/null ||
    fail "synchronization/parity gate is not green"
  jq -e '.ok == true and .result.projection_outbox_depth == 0 and
    .result.projection_lag == 0 and .result.projection_quarantined == 0 and
    .result.fully_synchronized == true' <<<"${writes}" >/dev/null ||
    fail "projection outbox has quarantined writes"
  # The permitted security_divergence count is exactly zero; the shadow gate
  # above is disabled whenever a security report diverges.
}

run_hook() {
  local variable=$1 description=$2
  local executable=${!variable:-}
  [[ -n "${executable}" ]] || fail "${variable} must name an executable for ${description}"
  [[ -x "${executable}" ]] || fail "hook is not executable: ${executable}"
  "${executable}"
}

run_phase() {
  case "${PHASE}" in
    install)
      compose config --quiet
      if [[ "${MIGRATING_REFERENCE}" == "true" ]]; then
        write_policy reference_only 0
      else
        write_policy primary_reads 10000
      fi
      ;;
    initial-import)
      require_reference_migration
      [[ -s "${OPENCTI_MIGRATION_BUNDLE:-}" ]] || fail "OPENCTI_MIGRATION_BUNDLE must point to a consistent STIX bundle"
      curl --fail --silent --show-error --max-time 3600 --cacert "${CA_FILE}" \
        --header "Authorization: Bearer $(token)" --header "Content-Type: application/json" \
        --data-binary "@${OPENCTI_MIGRATION_BUNDLE}" "${CORROBORE_URL}/v1/import/stix" >/dev/null
      ;;
    catch-up)
      require_reference_migration
      [[ -s "${OPENCTI_CATCH_UP_BATCH:-}" ]] || fail "OPENCTI_CATCH_UP_BATCH must point to a source-sequenced batch"
      api POST /v1/opencti/sync/batches "${OPENCTI_CATCH_UP_BATCH}" >/dev/null
      ;;
    validate)
      require_reference_migration
      assert_parity_gate
      run_hook OPENCTI_PARITY_VALIDATION_COMMAND "functional and security parity validation"
      ;;
    shadow)
      require_reference_migration
      assert_parity_gate
      write_policy shadow 0
      restart_corrobore
      ;;
    canary)
      require_reference_migration
      assert_parity_gate
      write_policy canary "${CORROBORE_CANARY_BASIS_POINTS:-500}"
      restart_corrobore
      ;;
    primary-read)
      require_reference_migration
      assert_parity_gate
      write_policy primary_reads 10000
      restart_corrobore
      ;;
    primary-write)
      require_reference_migration
      assert_parity_gate
      local payload
      payload="$(mktemp "${TMPDIR:-/tmp}/corrobore-authority.XXXXXX")"
      trap 'rm -f "${payload:-}"' RETURN
      jq -n '{target:"corrobore_primary",reference_healthy:true,replay_complete:true,parity_verified:true}' >"${payload}"
      api POST /v1/admin/opencti/authority "${payload}" >/dev/null
      ;;
    safety-delay)
      require_reference_migration
      assert_parity_gate
      ;;
    shutdown-elastic)
      require_reference_migration
      assert_parity_gate
      local started elapsed
      started="$(jq -r '.safety_delay_started_epoch // empty' "${STATE_FILE}")"
      [[ -n "${started}" ]] || fail "safety-delay has no durable start time"
      elapsed=$(( $(date +%s) - started ))
      [[ "${elapsed}" -ge "${SAFETY_DELAY}" ]] || fail "safety delay has ${SAFETY_DELAY}s minimum; ${elapsed}s elapsed"
      run_hook OPENCTI_REFERENCE_SHUTDOWN_COMMAND "verified Elasticsearch/OpenSearch shutdown"
      export CORROBORE_OPENCTI_ELASTIC_FREE=true
      write_policy primary_reads 10000
      docker compose -f "${COMPOSE_FILE}" up -d --no-deps --wait corrobore
      ;;
    *) fail "unknown phase: ${PHASE}" ;;
  esac
}

rollback() {
  local trigger=${CORROBORE_ROLLBACK_TRIGGER:-migration_failure}
  local suspend authority
  suspend="$(mktemp "${TMPDIR:-/tmp}/corrobore-suspend.XXXXXX")"
  authority="$(mktemp "${TMPDIR:-/tmp}/corrobore-rollback.XXXXXX")"
  trap 'rm -f "${suspend}" "${authority}"' RETURN
  jq -n --arg trigger "${trigger}" '{trigger:$trigger}' >"${suspend}"
  api POST /v1/admin/opencti/authority/suspend "${suspend}" >/dev/null
  run_hook OPENCTI_REFERENCE_RESTORE_COMMAND "reference restore and replay"
  jq -n '{target:"reference_primary",reference_healthy:true,replay_complete:true,parity_verified:true}' >"${authority}"
  api POST /v1/admin/opencti/authority "${authority}" >/dev/null
  write_policy reference_only 0
  restart_corrobore
  record_phase rollback
}

[[ -n "${PHASE}" ]] || { usage; exit 2; }
require_command jq
initialize_state
if command -v flock >/dev/null 2>&1; then
  exec 9>"${LOCK_FILE}"
  flock -n 9 || fail "another migration command owns ${LOCK_FILE}"
else
  LOCK_DIRECTORY="${LOCK_FILE}.d"
  mkdir "${LOCK_DIRECTORY}" 2>/dev/null || fail "another migration command owns ${LOCK_FILE}"
  trap 'rmdir "${LOCK_DIRECTORY}" 2>/dev/null || true' EXIT
fi

case "${PHASE}" in
  status) jq . "${STATE_FILE}" ;;
  rollback) rollback ;;
  *)
    require_command curl
    require_command docker
    assert_next_phase "${PHASE}"
    run_phase
    record_phase "${PHASE}"
    ;;
esac
