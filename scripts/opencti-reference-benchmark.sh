#!/usr/bin/env bash
set -euo pipefail

mode="${1:-all}"
case "$mode" in
  all|small|medium) ;;
  *) echo "usage: $0 [all|small|medium] [both|elasticsearch|opensearch]" >&2; exit 2 ;;
esac
engine_selection="${2:-both}"
case "$engine_selection" in
  both|elasticsearch|opensearch) ;;
  *) echo "usage: $0 [all|small|medium] [both|elasticsearch|opensearch]" >&2; exit 2 ;;
esac

container_name="corrobore-opencti-reference"
endpoint="http://127.0.0.1:19200"
results_dir="tmp/opencti-reference-benchmark"
mkdir -p "$results_dir"

cleanup() {
  podman rm --force "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

run_engine() {
  local engine_id="$1"
  local image="$2"
  shift 2

  cleanup
  podman run --detach --name "$container_name" \
    --publish 19200:9200 \
    --memory 4g \
    --env "OPENSEARCH_JAVA_OPTS=-Xms2g -Xmx2g" \
    --env "ES_JAVA_OPTS=-Xms2g -Xmx2g" \
    "$@" \
    "$image" >/dev/null

  local profiles=("$mode")
  if [[ "$mode" == "all" ]]; then
    profiles=(small medium)
  fi
  for profile in "${profiles[@]}"; do
    node scripts/opencti-reference-benchmark.mjs run \
      --endpoint "$endpoint" \
      --engine "$engine_id" \
      --profile "$profile" \
      --output "$results_dir/$engine_id-$profile.json"
  done
  cleanup
}

if [[ "$engine_selection" == "both" || "$engine_selection" == "elasticsearch" ]]; then
  run_engine \
    "elasticsearch-8.19.18" \
    "docker.elastic.co/elasticsearch/elasticsearch:8.19.18" \
    --env discovery.type=single-node \
    --env xpack.security.enabled=false
fi

if [[ "$engine_selection" == "both" || "$engine_selection" == "opensearch" ]]; then
  run_engine \
    "opensearch-3.7.0" \
    "opensearchproject/opensearch:3.7.0" \
    --env discovery.type=single-node \
    --env DISABLE_SECURITY_PLUGIN=true
fi

if [[ "$mode" == "all" && "$engine_selection" == "both" ]]; then
  node scripts/opencti-reference-benchmark.mjs merge \
    --output compatibility/opencti/7.260722.0/benchmark-results.json \
    --inputs "$results_dir/elasticsearch-8.19.18-small.json,$results_dir/elasticsearch-8.19.18-medium.json,$results_dir/opensearch-3.7.0-small.json,$results_dir/opensearch-3.7.0-medium.json"
fi
