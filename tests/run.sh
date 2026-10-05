#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build -p board-program --release --target wasm32-unknown-unknown
CARGO_TARGET_DIR="$ROOT/target/legacy" cargo build -p board-program --release --target wasm32-unknown-unknown --features legacy

cargo test --workspace

if command -v npm >/dev/null 2>&1; then
    (cd "$ROOT/examples/board-frontend" && npm install --no-audit --no-fund && npm test && npm run typecheck)
else
    echo "npm not found: skipping board-frontend schema checks" >&2
fi

exec "$ROOT/tests/services.sh" "$@"
