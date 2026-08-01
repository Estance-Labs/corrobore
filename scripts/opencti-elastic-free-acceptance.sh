#!/usr/bin/env bash

set -euo pipefail

MODE="${1:-contracts}"
ARTIFACT_DIR="${OPENCTI_ACCEPTANCE_ARTIFACT_DIR:-acceptance-artifacts/opencti-elastic-free}"
COMPOSE_FILE="${OPENCTI_CORROBORE_COMPOSE_FILE:-packaging/opencti-elastic-free/compose.yml}"
COMPOSE_ENV_FILE="${OPENCTI_CORROBORE_ENV_FILE:-}"
TIMEOUT_SECONDS="${OPENCTI_ACCEPTANCE_TIMEOUT_SECONDS:-300}"
PROFILE="${OPENCTI_ACCEPTANCE_PROFILE:-small}"
RUN_SUFFIX="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}-$$"
PROJECT_NAME="opencti-corrobore-acceptance-${RUN_SUFFIX}"

SUITES=(
  functional dashboard export traversal search aggregation file-content bulk
  merge concurrent-write durability security migration operations
  performance-small performance-medium
)

note() { printf '[opencti-acceptance:%s] %s\n' "${MODE}" "$*"; }
fail() { printf '[opencti-acceptance:%s] ERROR: %s\n' "${MODE}" "$*" >&2; exit 1; }
require_command() { command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"; }

compose() {
  local arguments=(--project-name "${PROJECT_NAME}" -f "${COMPOSE_FILE}")
  if [[ -n "${COMPOSE_ENV_FILE}" ]]; then
    arguments=(--env-file "${COMPOSE_ENV_FILE}" "${arguments[@]}")
  fi
  docker compose "${arguments[@]}" "$@"
}

cleanup() {
  local status=$?
  if [[ "${MODE}" == "stack" || "${MODE}" == "all" ]]; then
    if [[ ${status} -ne 0 ]]; then
      compose ps --all --format json >"${ARTIFACT_DIR}/compose-ps.json" 2>/dev/null || true
      compose logs --no-color >"${ARTIFACT_DIR}/compose.log" 2>&1 || true
    fi
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

run_contracts() {
  note "validating distribution contracts"
  node --test scripts/opencti-elastic-free-contract.test.mjs
  bash -n scripts/opencti-elastic-free-migrate.sh scripts/opencti-elastic-free-acceptance.sh scripts/opencti-elastic-free-demo-data.sh
  sh -n packaging/opencti-elastic-free/opencti-demo-data-entrypoint.sh
  python3 -c 'compile(open("packaging/opencti-elastic-free/opencti-demo-data-loader.py", encoding="utf-8").read(), "opencti-demo-data-loader.py", "exec")'
  compose config --quiet
  local rendered="${ARTIFACT_DIR}/compose-rendered.yml"
  compose config >"${rendered}"
  ! grep -Eq '^  (elasticsearch|opensearch):|ELASTICSEARCH__' "${rendered}" ||
    fail "rendered shipped stack contains an Elasticsearch/OpenSearch dependency"
}

run_local_matrix() {
  note "running Corrobore evidence mapped to OpenCTI suites"
  cargo test -p opencti-adapter --locked
  cargo test -p opencti-access --locked
  cargo test -p opencti-search --locked
  cargo test -p opencti-file-search --locked
  cargo test -p corrobore-engine --test opencti_core_reads --locked
  cargo test -p corrobore-engine --test opencti_authorization --locked
  cargo test -p corrobore-engine --test opencti_advanced_queries --locked
  cargo test -p corrobore-engine --test opencti_progressive_routing --locked
  cargo test -p corrobore-engine --test opencti_shadow_parity --locked
  cargo test -p corrobore-engine --test opencti_transactional_write_contract --locked
  cargo test -p corrobore-http-server --test opencti_primary_projection --locked
  cargo test -p corrobore-http-server --test opencti_elastic_free_contract --locked
  cargo test -p corrobore-http-server --test opencti_merge_durability --locked
  cargo test -p corrobore-http-server --test opencti_sync_contract --locked
  cargo test -p corrobore-http-server --test opencti_reconciliation_contract --locked
  cargo test -p graph-core --test deterministic_export_plan_acceptance --locked
  cargo test -p graph-core --test export_metadata_contract --locked
  cargo test -p graph-storage --test opencti_write_durability --locked
  cargo test -p graph-storage --test durability_acceptance_matrix --locked
  cargo test -p graph-storage --test graph_store_reopen_recovery --locked
  cargo test -p graph-storage --test torn_write_recovery --locked
  cargo test -p graph-storage --test backup_restore_integrity --locked
  cargo test -p graph-storage --test database_operations --locked
  node --test scripts/opencti-compatibility.test.mjs
  jq -n --args \
    '{schema_version:1,evidence:"corrobore-contract-and-pinned-opencti-runtime",suites:($ARGS.positional | map(
      if . == "performance-medium" then {
        suite:.,status:"conditional",gate:"publish and pass Corrobore measurements at 1,000,000 objects and 5,000,000 relationships"
      } else {suite:.,status:"passed"} end))}' "${SUITES[@]}" \
    >"${ARTIFACT_DIR}/matrix-evidence.json"
}

wait_for_opencti() {
  local started=$SECONDS
  until compose exec -T opencti node -e \
    "const k=require('node:fs').readFileSync('/run/secrets/opencti-health-key','utf8').trim();fetch('http://127.0.0.1:8080/health?health_access_key='+encodeURIComponent(k)).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"; do
    (( SECONDS - started < TIMEOUT_SECONDS )) || fail "OpenCTI did not become healthy within ${TIMEOUT_SECONDS}s"
    sleep 5
  done
}

capture_resources() {
  local stats="${ARTIFACT_DIR}/container-stats.json"
  local container_ids
  read -r -a container_ids <<<"$(compose ps -q | tr '\n' ' ')"
  docker stats --no-stream --format '{{json .}}' "${container_ids[@]}" >"${stats}"
  jq -n \
    --arg profile "${PROFILE}" \
    --argjson service_count "$(compose config --services | wc -l | tr -d ' ')" \
    --argjson mandatory_configuration "$(compose config | grep -Ec '^      [A-Z0-9_]+:' || true)" \
    --slurpfile containers "${stats}" \
    --slurpfile compatibility packaging/opencti-elastic-free/compatibility.json \
    'def memory_bytes:
       if type == "number" then .
       else capture("^(?<value>[0-9.]+)(?<unit>[KMGT]?i?B)") as $m
       | ($m.value | tonumber) * ({B:1,KB:1000,KiB:1024,MB:1000000,MiB:1048576,GB:1000000000,GiB:1073741824,TB:1000000000000,TiB:1099511627776}[$m.unit])
       end;
     ($containers | map(.MemUsage | memory_bytes) | add // 0 | floor) as $memory_bytes
     | $compatibility[0].reference_stack as $reference
     | {schema_version:1,profile:$profile,service_count:$service_count,mandatory_configuration:$mandatory_configuration,memory_bytes:$memory_bytes,containers:$containers,
        comparison:{reference:$reference,
          service_count_reduction:($reference.service_count - $service_count),
          mandatory_configuration_reduction:($reference.mandatory_configuration - $mandatory_configuration),
          memory_reduction_lower_bound_bytes:($reference.memory_lower_bound_bytes - $memory_bytes),
          passed:($service_count < $reference.service_count and
            $mandatory_configuration < $reference.mandatory_configuration and
            $memory_bytes < $reference.memory_lower_bound_bytes)}}' \
    >"${ARTIFACT_DIR}/resource-evidence.json"
}

validate_opencti_runtime() {
  local startup_seconds=$1
  local graphql_result="${ARTIFACT_DIR}/opencti-graphql.json"
  compose exec -T opencti node -e '
    const fs = require("node:fs");
    const token = fs.readFileSync("/run/secrets/opencti-admin-token", "utf8").trim();
    const elastic = Object.keys(process.env).filter((key) => key.startsWith("ELASTICSEARCH__"));
    if (process.env.DATABASE_ENGINE !== "corrobore" || elastic.length !== 0) process.exit(2);
    fetch("http://127.0.0.1:8080/graphql", {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({ query: "query AcceptanceRuntime { me { id user_email } settings { id } stixCoreRelationships(first: 1) { edges { node { id relationship_type } } } }" }),
    }).then(async (response) => {
      const payload = await response.json();
      if (!response.ok || payload.errors?.length || !payload.data?.me?.id || !payload.data?.settings?.id || !payload.data?.stixCoreRelationships?.edges) process.exit(3);
      process.stdout.write(JSON.stringify({ database_engine: process.env.DATABASE_ENGINE, elastic_variables: elastic, graphql: payload }));
    }).catch(() => process.exit(4));
  ' >"${graphql_result}"
  jq -n \
    --argjson startup_seconds "${startup_seconds}" \
    --slurpfile runtime "${graphql_result}" \
    '{schema_version:1,startup_seconds:$startup_seconds,database_engine:$runtime[0].database_engine,elastic_variables:$runtime[0].elastic_variables,admin_query_succeeded:($runtime[0].graphql.data.me.id | length > 0),settings_query_succeeded:($runtime[0].graphql.data.settings.id | length > 0),relationship_query_succeeded:($runtime[0].graphql.data.stixCoreRelationships.edges | type == "array")}' \
    >"${ARTIFACT_DIR}/runtime-evidence.json"
}

run_stack() {
  require_command docker
  note "building and starting the exact Elastic-free stack"
  compose build --pull corrobore opencti
  local started=$SECONDS
  compose up -d --wait
  local startup_seconds=$((SECONDS - started))
  wait_for_opencti
  OPENCTI_CORROBORE_COMPOSE_FILE="${COMPOSE_FILE}" \
    OPENCTI_CORROBORE_ENV_FILE="${COMPOSE_ENV_FILE}" \
    OPENCTI_CORROBORE_PROJECT_NAME="${PROJECT_NAME}" \
    scripts/opencti-elastic-free-demo-data.sh
  compose ps --services --status running >"${ARTIFACT_DIR}/running-services.txt"
  [[ "$(grep -Ec '^(opencti|worker|corrobore|file-worker|redis|rabbitmq|minio)$' "${ARTIFACT_DIR}/running-services.txt")" -eq 7 ]] ||
    fail "not every supported service is running"
  validate_opencti_runtime "${startup_seconds}"
  capture_resources
}

mkdir -p "${ARTIFACT_DIR}"
require_command node
require_command jq

case "${MODE}" in
  contracts) run_contracts ;;
  local) run_local_matrix ;;
  stack) run_contracts; run_stack ;;
  all) run_contracts; run_local_matrix; run_stack ;;
  *) fail "usage: $0 <contracts|local|stack|all>" ;;
esac
