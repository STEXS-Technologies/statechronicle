#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"

echo "[1/2] SQLite recovery and integrity drill"
cargo test -p statechronicle-sqlite --all-targets --all-features --locked --quiet

echo "[2/2] PostgreSQL transactional deployment drill"
./scripts/run_postgres_integration.sh

echo "deployment drill passed"
