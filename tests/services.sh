#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STATE=/tmp/echtest
mkdir -p "$STATE"

FDB=ech-test-fdb
S3=ech-test-s3

cleanup() {
    docker rm -f "$FDB" "$S3" >/dev/null 2>&1 || :
}
trap cleanup EXIT
cleanup

docker run -d --name "$FDB" --network host \
    -e FDB_NETWORKING_MODE=host \
    -e FDB_PORT=4500 \
    --tmpfs /var/fdb/data:size=2g \
    foundationdb/foundationdb:7.3.63
timeout 60 bash -c "until docker exec $FDB cat /var/fdb/fdb.cluster > /dev/null 2>&1; do sleep 1; done"
docker exec "$FDB" cat /var/fdb/fdb.cluster > "$STATE/fdb.cluster"
timeout 120 bash -c "until fdbcli -C $STATE/fdb.cluster --timeout 5 --exec 'configure new single ssd' || fdbcli -C $STATE/fdb.cluster --timeout 5 --exec 'status minimal'; do sleep 1; done"
sleep 3

docker run -d --name "$S3" --network host \
    --tmpfs /data:size=1g \
    chrislusf/seaweedfs:3.80 \
    server -s3 -dir=/data -s3.port=8333 -master.port=9333 -volume.port=8080 -filer.port=8888
timeout 60 bash -c 'until [ "$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8333)" != "000" ]; do sleep 1; done'
sleep 3

export ECH_TEST_FDB_CLUSTER_FILE="$STATE/fdb.cluster"
export ECH_TEST_S3_ENDPOINT=http://127.0.0.1:8333
export ECH_TEST_S3_REGION=us-east-1
export ECH_TEST_S3_BUCKET=ech-db
export ECH_TEST_S3_ACCESS_KEY=ech-access
export ECH_TEST_S3_SECRET_KEY=ech-secret

if [ "$#" -gt 0 ]; then
    "$@"
else
    cargo test -p ech-db-entity-tests -- --ignored --test-threads=1
fi
