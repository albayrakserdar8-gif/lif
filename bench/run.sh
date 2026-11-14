#!/bin/sh
# Builds the driver and runs the benchmark suite.
#   bench/run.sh [--tools none,tmux,zellij,lif] [--quick] [--only throughput,latency] [--tag x]
# Results: bench/results/<date>-<tool>.json and <date>-summary.md
set -eu
export PATH=/opt/homebrew/bin:$PATH
here=$(cd "$(dirname "$0")" && pwd)
: "${CARGO_TARGET_DIR:=$(dirname "$here")/target-bench}"
export CARGO_TARGET_DIR
cargo build --release --quiet --manifest-path "$here/driver/Cargo.toml"
exec python3 "$here/lifbench.py" run "$@"
