#!/bin/sh
set -eu

# Release-only acceptance harness for the actual production image.

IMAGE="${IMAGE:-corrobore-smoke:local}"
CONTAINER_ENGINE="${CONTAINER_ENGINE:-docker}"
CORROBORE_BUILD_VERSION="${CORROBORE_BUILD_VERSION:-dev}"
CORROBORE_BUILD_REVISION="${CORROBORE_BUILD_REVISION:-unknown}"
SMOKE_PORT="${SMOKE_PORT:-18080}"
SMOKE_ROOT="$(mktemp -d)"
SMOKE_VOLUME="corrobore-smoke-${GITHUB_RUN_ID:-$$}"
SMOKE_CONTAINER="corrobore-smoke-${GITHUB_RUN_ID:-$$}"
SMOKE_TOKEN="container-smoke-secret"

cleanup() {
  "${CONTAINER_ENGINE}" rm -f "${SMOKE_CONTAINER}" >/dev/null 2>&1 || true
  "${CONTAINER_ENGINE}" volume rm "${SMOKE_VOLUME}" >/dev/null 2>&1 || true
  rm -rf "${SMOKE_ROOT}"
}

prepare_fixtures() {
  openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
    -keyout "${SMOKE_ROOT}/tls.key" \
    -out "${SMOKE_ROOT}/tls.crt" \
    -days 1 \
    -subj "/CN=localhost" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" >/dev/null 2>&1
  printf '%s\n' "${SMOKE_TOKEN}" >"${SMOKE_ROOT}/token"
  # Bind-mounted fixtures must be readable by the image's uid 65532. The
  # temporary directory is private and removed by the EXIT trap.
  chmod 0444 "${SMOKE_ROOT}/tls.crt" "${SMOKE_ROOT}/tls.key" "${SMOKE_ROOT}/token"
  "${CONTAINER_ENGINE}" volume create "${SMOKE_VOLUME}" >/dev/null
}

start_container() {
  "${CONTAINER_ENGINE}" run --detach \
    --name "${SMOKE_CONTAINER}" \
    --publish "127.0.0.1:${SMOKE_PORT}:8080" \
    --volume "${SMOKE_VOLUME}:/data" \
    --volume "${SMOKE_ROOT}/token:/run/secrets/corrobore-http-token:ro" \
    --volume "${SMOKE_ROOT}/tls.crt:/run/secrets/tls.crt:ro" \
    --volume "${SMOKE_ROOT}/tls.key:/run/secrets/tls.key:ro" \
    "${IMAGE}" >/dev/null

  attempt=0
  while [ "${attempt}" -lt 60 ]; do
    health="$("${CONTAINER_ENGINE}" inspect --format '{{.State.Health.Status}}' "${SMOKE_CONTAINER}")"
    [ "${health}" = "healthy" ] && return 0
    [ "${health}" = "unhealthy" ] && {
      "${CONTAINER_ENGINE}" inspect --format '{{json .State.Health}}' "${SMOKE_CONTAINER}"
      "${CONTAINER_ENGINE}" logs "${SMOKE_CONTAINER}"
      return 1
    }
    attempt=$((attempt + 1))
    sleep 2
  done
  "${CONTAINER_ENGINE}" inspect --format '{{json .State.Health}}' "${SMOKE_CONTAINER}"
  "${CONTAINER_ENGINE}" logs "${SMOKE_CONTAINER}"
  return 1
}

authenticated_curl() {
  curl --fail --silent --show-error --insecure \
    --header "Authorization: Bearer ${SMOKE_TOKEN}" "$@"
}

verify_release_contract() {
  [ "$("${CONTAINER_ENGINE}" inspect --format '{{.Config.User}}' "${SMOKE_CONTAINER}")" = "65532:65532" ]
  [ "$("${CONTAINER_ENGINE}" inspect --format '{{ index .Config.Labels "org.opencontainers.image.version" }}' "${IMAGE}")" = "${CORROBORE_BUILD_VERSION}" ]
  [ "$("${CONTAINER_ENGINE}" inspect --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}' "${IMAGE}")" = "${CORROBORE_BUILD_REVISION}" ]

  authenticated_curl "https://127.0.0.1:${SMOKE_PORT}/health/ready" |
    grep -q '"ready":true'
  authenticated_curl \
    --header "Content-Type: application/json" \
    --request POST \
    --data '{"query":"CREATE (n:Smoke {name: '\''container-persistence'\''})"}' \
    "https://127.0.0.1:${SMOKE_PORT}/v1/cypher/write" |
    grep -q '"ok":true'
  read_response="$(authenticated_curl \
    --header "Content-Type: application/json" \
    --request POST \
    --data '{"query":"MATCH (n:Smoke) RETURN n"}' \
    "https://127.0.0.1:${SMOKE_PORT}/v1/cypher/read")"
  printf '%s\n' "${read_response}" | grep -q '"n":"node--' || {
    printf 'written node was not returned: %s\n' "${read_response}" >&2
    return 1
  }

  session_response="$(authenticated_curl \
    --header "Content-Type: application/json" \
    --request POST \
    --data '{"workspace_id":"workspace--container-persistence","actor_id":"actor--container-smoke","actor_kind":"agent"}' \
    "https://127.0.0.1:${SMOKE_PORT}/v1/sessions/start")"
  session_id="$(printf '%s\n' "${session_response}" |
    sed -n 's/.*"session_id":"\([^"]*\)".*/\1/p')"
  [ -n "${session_id}" ] || {
    printf 'session id was not returned: %s\n' "${session_response}" >&2
    return 1
  }

  "${CONTAINER_ENGINE}" stop --time 15 "${SMOKE_CONTAINER}" >/dev/null
  "${CONTAINER_ENGINE}" rm "${SMOKE_CONTAINER}" >/dev/null
  start_container

  persisted_response="$(authenticated_curl \
    "https://127.0.0.1:${SMOKE_PORT}/v1/sessions/${session_id}/health")"
  printf '%s\n' "${persisted_response}" |
    grep -q 'workspace--container-persistence' || {
    printf 'persisted session was not returned: %s\n' "${persisted_response}" >&2
    return 1
  }
}

trap cleanup EXIT
prepare_fixtures
start_container
verify_release_contract

echo "container smoke test passed"
