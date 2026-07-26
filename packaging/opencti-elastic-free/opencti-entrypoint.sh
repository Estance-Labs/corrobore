#!/bin/sh
set -eu

read_secret() {
  variable=$1
  path=$2
  value=$(cat "${path}")
  if [ -z "${value}" ]; then
    echo "required secret is empty: ${path}" >&2
    exit 2
  fi
  export "${variable}=${value}"
}

read_secret APP__ADMIN__PASSWORD /run/secrets/opencti-admin-password
read_secret APP__ADMIN__TOKEN /run/secrets/opencti-admin-token
read_secret APP__HEALTH_ACCESS_KEY /run/secrets/opencti-health-key
read_secret APP__ENCRYPTION_KEY /run/secrets/opencti-encryption-key
read_secret RABBITMQ__PASSWORD /run/secrets/rabbitmq-password
read_secret MINIO__SECRET_KEY /run/secrets/minio-secret-key

exec /sbin/tini -- "$@"
