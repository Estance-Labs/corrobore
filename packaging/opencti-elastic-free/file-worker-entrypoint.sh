#!/bin/sh
set -eu

secret=$(cat /run/secrets/minio-secret-key)
if [ -z "${secret}" ]; then
  echo "MinIO secret is empty" >&2
  exit 2
fi

export MC_CONFIG_DIR=/tmp/mc
until mc alias set opencti "${MINIO_ENDPOINT}" "${MINIO_ACCESS_KEY}" "${secret}" >/dev/null 2>&1 \
  && mc stat "opencti/${MINIO_BUCKET}" >/dev/null 2>&1; do
  sleep 2
done

mc mirror --overwrite --watch "opencti/${MINIO_BUCKET}" /opencti-files &
mirror_pid=$!
trap 'kill "${mirror_pid}" 2>/dev/null || true' EXIT INT TERM

exec /usr/local/bin/corrobore-file-worker
