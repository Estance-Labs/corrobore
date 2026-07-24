#!/usr/bin/env bash

set -euo pipefail

# End-to-end acceptance harness for the real standalone product. Native and
# container adapters feed the same HTTP assertions so protocol behavior cannot
# drift between supported deployment forms.

MODE="${1:-}"
TIMEOUT_SECONDS="${CORROBORE_ACCEPTANCE_TIMEOUT_SECONDS:-30}"
ARTIFACT_ROOT="${CORROBORE_ACCEPTANCE_ARTIFACT_DIR:-acceptance-artifacts}"
NATIVE_BINARY="${CORROBORE_ACCEPTANCE_BINARY:-target/release/corrobore}"
CONTAINER_ENGINE="${CONTAINER_ENGINE:-docker}"
CONTAINER_IMAGE="${IMAGE:-corrobore-acceptance:local}"
PORT="${CORROBORE_ACCEPTANCE_PORT:-18084}"
TOKEN="standalone-acceptance-secret"
CORRELATION_ID="standalone-acceptance-correlation"
RUN_SUFFIX="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}-$$"
CONTAINER_NAME="corrobore-acceptance-${RUN_SUFFIX}"
SECONDARY_CONTAINER_NAME="${CONTAINER_NAME}-secondary"
CONTAINER_VOLUME="corrobore-acceptance-${RUN_SUFFIX}"
WORK_ROOT=""
ACTIVE_NATIVE_PID=""
FAILURE=0

usage() {
  echo "usage: $0 <native|container>" >&2
}

note() {
  printf '[acceptance:%s] %s\n' "${MODE}" "$*"
}

fail() {
  FAILURE=1
  printf '[acceptance:%s] ERROR: %s\n' "${MODE}" "$*" >&2
  return 1
}

cleanup() {
  local status=$?
  if [[ -n "${ACTIVE_NATIVE_PID}" ]] && kill -0 "${ACTIVE_NATIVE_PID}" 2>/dev/null; then
    kill -TERM "${ACTIVE_NATIVE_PID}" 2>/dev/null || true
    wait "${ACTIVE_NATIVE_PID}" 2>/dev/null || true
  fi
  if [[ "${MODE}" == "container" ]]; then
    if "${CONTAINER_ENGINE}" inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
      "${CONTAINER_ENGINE}" inspect "${CONTAINER_NAME}" \
        >"${ARTIFACT_ROOT}/cleanup-container-inspect.json" 2>/dev/null || true
      "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" \
        >"${ARTIFACT_ROOT}/cleanup-container.stdout" \
        2>"${ARTIFACT_ROOT}/cleanup-container.stderr" || true
    fi
    "${CONTAINER_ENGINE}" rm -f "${SECONDARY_CONTAINER_NAME}" >/dev/null 2>&1 || true
    "${CONTAINER_ENGINE}" rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
    "${CONTAINER_ENGINE}" volume rm "${CONTAINER_VOLUME}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${WORK_ROOT}" && -d "${WORK_ROOT}" ]]; then
    if [[ ${status} -ne 0 || ${FAILURE} -ne 0 ]]; then
      mkdir -p "${ARTIFACT_ROOT}/workspace"
      cp "${WORK_ROOT}/corrobore.toml" "${ARTIFACT_ROOT}/workspace/" 2>/dev/null || true
      cp "${WORK_ROOT}/invalid.toml" "${ARTIFACT_ROOT}/workspace/" 2>/dev/null || true
      cp "${WORK_ROOT}/tls.crt" "${ARTIFACT_ROOT}/workspace/" 2>/dev/null || true
      cp -R "${WORK_ROOT}/logs" "${ARTIFACT_ROOT}/workspace/" 2>/dev/null || true
    fi
    rm -rf "${WORK_ROOT}"
  fi
}

trap cleanup EXIT

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

prepare_acceptance_workspace() {
  mkdir -p "${ARTIFACT_ROOT}"
  WORK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/corrobore-acceptance.XXXXXX")"
  mkdir -p \
    "${WORK_ROOT}/runtime" \
    "${WORK_ROOT}/logs" \
    "${ARTIFACT_ROOT}"

  openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
    -keyout "${WORK_ROOT}/tls.key" \
    -out "${WORK_ROOT}/tls.crt" \
    -days 1 \
    -subj "/CN=localhost" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
    >"${ARTIFACT_ROOT}/openssl.stdout" 2>"${ARTIFACT_ROOT}/openssl.stderr"
  printf '%s\n' "${TOKEN}" >"${WORK_ROOT}/token"
  chmod 0600 "${WORK_ROOT}/token" "${WORK_ROOT}/tls.key"
  chmod 0644 "${WORK_ROOT}/tls.crt"
}

deadline_epoch() {
  echo $(( $(date +%s) + TIMEOUT_SECONDS ))
}

wait_for_ready() {
  local deadline
  deadline="$(deadline_epoch)"
  until authenticated_curl "/health/ready" >"${ARTIFACT_ROOT}/readiness.json" 2>/dev/null; do
    if [[ "${MODE}" == "native" && -n "${ACTIVE_NATIVE_PID}" ]] &&
      ! kill -0 "${ACTIVE_NATIVE_PID}" 2>/dev/null; then
      fail "native process exited before readiness; inspect native.stderr"
      return
    fi
    if [[ "${MODE}" == "container" ]] &&
      [[ "$("${CONTAINER_ENGINE}" inspect --format '{{.State.Running}}' "${CONTAINER_NAME}" 2>/dev/null || echo false)" != "true" ]]; then
      fail "container exited before readiness; inspect container diagnostics"
      return
    fi
    if (( $(date +%s) >= deadline )); then
      fail "readiness did not become true within ${TIMEOUT_SECONDS}s"
      return
    fi
    sleep 1
  done
  grep -q '"ready":true' "${ARTIFACT_ROOT}/readiness.json" ||
    fail "readiness response did not report ready=true"
}

authenticated_curl() {
  local path=$1
  shift
  curl --fail --silent --show-error \
    --max-time 5 \
    --cacert "${WORK_ROOT}/tls.crt" \
    --header "Authorization: Bearer ${TOKEN}" \
    "$@" "https://127.0.0.1:${PORT}${path}"
}

assert_authentication_required() {
  local status
  status="$(curl --silent --output "${ARTIFACT_ROOT}/unauthenticated.json" \
    --write-out '%{http_code}' \
    --max-time 5 \
    --cacert "${WORK_ROOT}/tls.crt" \
    "https://127.0.0.1:${PORT}/v1/cypher/read")"
  [[ "${status}" == "401" ]] ||
    fail "TLS/auth policy expected HTTP 401 without a bearer token, received ${status}"
}

# Shared public HTTP contract used by the native binary and production image.
exercise_public_http_contract() {
  local phase=$1
  authenticated_curl "/health/live" >"${ARTIFACT_ROOT}/${phase}-liveness.json"
  authenticated_curl "/health/ready" >"${ARTIFACT_ROOT}/${phase}-readiness.json"
  authenticated_curl "/version" >"${ARTIFACT_ROOT}/${phase}-version.json"
  grep -q '"live":true' "${ARTIFACT_ROOT}/${phase}-liveness.json"
  grep -q '"ready":true' "${ARTIFACT_ROOT}/${phase}-readiness.json"
  grep -q '"storage_compatibility"' "${ARTIFACT_ROOT}/${phase}-version.json"
  assert_authentication_required

  if [[ "${phase}" == "initial" ]]; then
    authenticated_curl "/v1/cypher/write" \
      --header "Content-Type: application/json" \
      --header "X-Request-Id: ${CORRELATION_ID}" \
      --request POST \
      --data '{"query":"CREATE (n:Acceptance {name: '\''persistent restart'\''})"}' \
      --dump-header "${ARTIFACT_ROOT}/correlation.headers" \
      >"${ARTIFACT_ROOT}/write.json"
    grep -qi "^x-request-id: ${CORRELATION_ID}" "${ARTIFACT_ROOT}/correlation.headers"
    grep -q '"ok":true' "${ARTIFACT_ROOT}/write.json"
  fi

  authenticated_curl "/v1/cypher/read" \
    --header "Content-Type: application/json" \
    --request POST \
    --data '{"query":"MATCH (n:Acceptance) RETURN n"}' \
    >"${ARTIFACT_ROOT}/${phase}-read.json"
  grep -q '"n":"node--' "${ARTIFACT_ROOT}/${phase}-read.json" ||
    fail "persistent acceptance node was not returned during ${phase}"
}

wait_for_process_exit() {
  local pid=$1
  local deadline
  deadline="$(deadline_epoch)"
  while kill -0 "${pid}" 2>/dev/null; do
    if (( $(date +%s) >= deadline )); then
      return 1
    fi
    sleep 1
  done
}

write_native_config() {
  cat >"${WORK_ROOT}/corrobore.toml" <<EOF
[server]
host = "127.0.0.1"
port = 18082
auth_mode = "required"
auth_token_file = "${WORK_ROOT}/token"
data_directory = "${WORK_ROOT}/runtime"
shutdown_timeout_ms = 10000

[storage]
mode = "persistent"
directory = "${WORK_ROOT}/graph"
require_fsync = true
strict_recovery = true

[logging]
directory = "${WORK_ROOT}/logs"
level = "info"
format = "json"

[operations]
endpoint_policy = "authenticated"

[tls]
enabled = true
certificate_file = "${WORK_ROOT}/tls.crt"
private_key_file = "${WORK_ROOT}/tls.key"
EOF
}

verify_native_configuration_contract() {
  local invalid_status
  cat >"${WORK_ROOT}/invalid.toml" <<EOF
[server]
host = "0.0.0.0"
port = 8080
EOF

  set +e
  "${NATIVE_BINARY}" server validate-config --config "${WORK_ROOT}/invalid.toml" \
    >"${ARTIFACT_ROOT}/invalid-config.stdout" \
    2>"${ARTIFACT_ROOT}/invalid-config.stderr"
  invalid_status=$?
  set -e
  [[ ${invalid_status} -eq 2 ]] ||
    fail "invalid configuration should exit 2, received ${invalid_status}"
  grep -q 'configuration error:' "${ARTIFACT_ROOT}/invalid-config.stderr"

  CORROBORE_HTTP_PORT=18083 \
    "${NATIVE_BINARY}" server validate-config \
      --config "${WORK_ROOT}/corrobore.toml" \
      --port "${PORT}" \
      --print-effective \
      >"${ARTIFACT_ROOT}/effective-config.txt"
  grep -q "server.port = ${PORT}" "${ARTIFACT_ROOT}/effective-config.txt" ||
    fail "configuration precedence did not apply CLI over environment and TOML"
  ! grep -q "${TOKEN}" "${ARTIFACT_ROOT}/effective-config.txt" ||
    fail "effective configuration exposed the configured secret"
}

start_native() {
  CORROBORE_HTTP_PORT=18083 \
    "${NATIVE_BINARY}" server start \
      --config "${WORK_ROOT}/corrobore.toml" \
      --port "${PORT}" \
      >>"${ARTIFACT_ROOT}/native.stdout" \
      2>>"${ARTIFACT_ROOT}/native.stderr" &
  ACTIVE_NATIVE_PID=$!
  wait_for_ready
}

stop_native_gracefully() {
  local pid="${ACTIVE_NATIVE_PID}"
  local started=$SECONDS
  kill -TERM "${pid}"
  wait_for_process_exit "${pid}" ||
    fail "SIGTERM did not stop the native process within ${TIMEOUT_SECONDS}s"
  set +e
  wait "${pid}"
  local status=$?
  set -e
  ACTIVE_NATIVE_PID=""
  [[ ${status} -eq 0 ]] || fail "native SIGTERM exit code was ${status}, expected 0"
  (( SECONDS - started <= TIMEOUT_SECONDS )) ||
    fail "native SIGTERM exceeded the configured acceptance bound"
}

verify_native_exclusive_ownership() {
  local secondary_pid secondary_status
  CORROBORE_HTTP_PORT=18085 \
    "${NATIVE_BINARY}" server start \
      --config "${WORK_ROOT}/corrobore.toml" \
      --port 18085 \
      >"${ARTIFACT_ROOT}/ownership.stdout" \
      2>"${ARTIFACT_ROOT}/ownership.stderr" &
  secondary_pid=$!
  if ! wait_for_process_exit "${secondary_pid}"; then
    kill -TERM "${secondary_pid}" 2>/dev/null || true
    wait "${secondary_pid}" 2>/dev/null || true
    fail "exclusive ownership check allowed a second native process to remain running"
  fi
  set +e
  wait "${secondary_pid}"
  secondary_status=$?
  set -e
  [[ ${secondary_status} -eq 4 ]] ||
    fail "exclusive ownership should exit 4, received ${secondary_status}"
  grep -qi 'ownership' "${ARTIFACT_ROOT}/ownership.stderr"
}

verify_logs_are_safe() {
  local structured_log="${ARTIFACT_ROOT}/http-server.session.log.jsonl"
  [[ -f "${structured_log}" ]] || fail "structured log artifact is missing"
  grep -q "${CORRELATION_ID}" "${structured_log}" ||
    fail "structured logs do not contain the correlation identifier"
  if grep -R -F "${TOKEN}" "${ARTIFACT_ROOT}" \
    --exclude='standalone-acceptance.sh' >/dev/null 2>&1; then
    fail "acceptance diagnostics contain the configured secret"
  fi
}

run_native_acceptance() {
  [[ -x "${NATIVE_BINARY}" ]] ||
    fail "native binary is not executable: ${NATIVE_BINARY}"
  write_native_config
  verify_native_configuration_contract
  note "configuration precedence and invalid configuration checks passed"
  start_native
  exercise_public_http_contract initial
  verify_native_exclusive_ownership
  note "exclusive ownership check passed"
  stop_native_gracefully
  start_native
  exercise_public_http_contract restarted
  note "persistent restart check passed"
  stop_native_gracefully
  cp "${WORK_ROOT}/logs/http-server.session.log.jsonl" \
    "${ARTIFACT_ROOT}/http-server.session.log.jsonl"
  verify_logs_are_safe
  note "SIGTERM, correlation identifier, and configured secret checks passed"
}

container_args() {
  printf '%s\n' \
    --volume "${CONTAINER_VOLUME}:/data" \
    --volume "${WORK_ROOT}/token:/run/secrets/corrobore-http-token:ro" \
    --volume "${WORK_ROOT}/tls.crt:/run/secrets/tls.crt:ro" \
    --volume "${WORK_ROOT}/tls.key:/run/secrets/tls.key:ro"
}

start_container() {
  local args=()
  while IFS= read -r value; do args+=("${value}"); done < <(container_args)
  "${CONTAINER_ENGINE}" run --detach \
    --name "${CONTAINER_NAME}" \
    --publish "127.0.0.1:${PORT}:8080" \
    "${args[@]}" \
    "${CONTAINER_IMAGE}" >/dev/null
  wait_for_ready
}

stop_container_gracefully() {
  local started=$SECONDS
  "${CONTAINER_ENGINE}" stop --time "${TIMEOUT_SECONDS}" "${CONTAINER_NAME}" \
    >"${ARTIFACT_ROOT}/container-stop.stdout"
  local exit_code
  exit_code="$("${CONTAINER_ENGINE}" inspect --format '{{.State.ExitCode}}' "${CONTAINER_NAME}")"
  [[ "${exit_code}" == "0" ]] ||
    fail "container SIGTERM exit code was ${exit_code}, expected 0"
  (( SECONDS - started <= TIMEOUT_SECONDS + 2 )) ||
    fail "container SIGTERM exceeded the configured acceptance bound"
}

verify_container_exclusive_ownership() {
  local args=()
  while IFS= read -r value; do args+=("${value}"); done < <(container_args)
  "${CONTAINER_ENGINE}" run --detach \
    --name "${SECONDARY_CONTAINER_NAME}" \
    "${args[@]}" \
    "${CONTAINER_IMAGE}" >/dev/null

  local deadline
  deadline="$(deadline_epoch)"
  while [[ "$("${CONTAINER_ENGINE}" inspect --format '{{.State.Running}}' "${SECONDARY_CONTAINER_NAME}")" == "true" ]]; do
    if (( $(date +%s) >= deadline )); then
      fail "exclusive ownership check allowed a second container to remain running"
      return
    fi
    sleep 1
  done
  local exit_code
  exit_code="$("${CONTAINER_ENGINE}" inspect --format '{{.State.ExitCode}}' "${SECONDARY_CONTAINER_NAME}")"
  "${CONTAINER_ENGINE}" logs "${SECONDARY_CONTAINER_NAME}" \
    >"${ARTIFACT_ROOT}/ownership.stdout" \
    2>"${ARTIFACT_ROOT}/ownership.stderr" || true
  [[ "${exit_code}" == "4" ]] ||
    fail "container exclusive ownership should exit 4, received ${exit_code}"
  grep -qi 'ownership' "${ARTIFACT_ROOT}/ownership.stderr"
  "${CONTAINER_ENGINE}" rm "${SECONDARY_CONTAINER_NAME}" >/dev/null
}

run_container_acceptance() {
  "${CONTAINER_ENGINE}" image inspect "${CONTAINER_IMAGE}" >/dev/null
  # Bind-mounted fixtures must be readable by the non-root uid 65532.
  chmod 0444 "${WORK_ROOT}/token" "${WORK_ROOT}/tls.key" "${WORK_ROOT}/tls.crt"
  "${CONTAINER_ENGINE}" volume create "${CONTAINER_VOLUME}" >/dev/null
  start_container
  exercise_public_http_contract initial
  verify_container_exclusive_ownership
  note "exclusive ownership check passed"
  stop_container_gracefully
  "${CONTAINER_ENGINE}" start "${CONTAINER_NAME}" >/dev/null
  wait_for_ready
  exercise_public_http_contract restarted
  note "persistent restart check passed"
  stop_container_gracefully
  "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" \
    >"${ARTIFACT_ROOT}/container.stdout" \
    2>"${ARTIFACT_ROOT}/container.stderr" || true
  "${CONTAINER_ENGINE}" cp \
    "${CONTAINER_NAME}:/data/logs/http-server.session.log.jsonl" \
    "${ARTIFACT_ROOT}/http-server.session.log.jsonl"
  verify_logs_are_safe
  note "SIGTERM, correlation identifier, and configured secret checks passed"
}

main() {
  if [[ "${MODE}" != "native" && "${MODE}" != "container" ]]; then
    usage
    return 64
  fi
  [[ "${TIMEOUT_SECONDS}" =~ ^[1-9][0-9]*$ ]] ||
    fail "CORROBORE_ACCEPTANCE_TIMEOUT_SECONDS must be a positive integer"

  require_command curl
  require_command openssl
  if [[ "${MODE}" == "container" ]]; then require_command "${CONTAINER_ENGINE}"; fi
  prepare_acceptance_workspace
  note "starting bounded ${MODE} acceptance"
  if [[ "${MODE}" == "native" ]]; then
    run_native_acceptance
  else
    run_container_acceptance
  fi
  note "all ${MODE} acceptance checks passed"
}

main
