#!/bin/sh

set -eu

# In-container loader boundary. It accepts only the curated dataset copied
# into this distribution, loads the administrator token from its Docker secret
# into the child environment, and propagates the pinned OpenCTI importer status.

DEFAULT_DATASETS="corrobore-demo"
DATASETS="${1:-${OPENCTI_DEMO_DATASETS:-${DEFAULT_DATASETS}}}"
TOKEN_FILE="${OPENCTI_ADMIN_TOKEN_FILE:-/run/secrets/opencti-admin-token}"

fail() {
  printf 'demo data error: %s\n' "$*" >&2
  exit 1
}

[ "$#" -le 1 ] || fail "expected one comma-separated dataset selection"
case "${DATASETS}" in
  ""|,*|*,|*,,*) fail "unsupported demo dataset selection: ${DATASETS}" ;;
esac

remaining="${DATASETS}"
while [ -n "${remaining}" ]; do
  case "${remaining}" in
    *,*)
      dataset=${remaining%%,*}
      remaining=${remaining#*,}
      ;;
    *)
      dataset=${remaining}
      remaining=""
      ;;
  esac
  case "${dataset}" in
    corrobore-demo) ;;
    *) fail "unsupported demo dataset: ${dataset}" ;;
  esac
done

[ -s "${TOKEN_FILE}" ] || fail "missing OpenCTI administrator token secret: ${TOKEN_FILE}"
APP__ADMIN__TOKEN=$(tr -d '\r\n' <"${TOKEN_FILE}")
[ -n "${APP__ADMIN__TOKEN}" ] || fail "OpenCTI administrator token secret is empty"
export APP__ADMIN__TOKEN

python3 /usr/local/lib/opencti-demo-data-loader.py "${DATASETS}" ||
  fail "OpenCTI demo data import failed"
