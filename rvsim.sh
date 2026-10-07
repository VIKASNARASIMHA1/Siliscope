#!/bin/sh
# Linux / macOS launcher.  Usage: ./rvsim.sh <build | run | selftest | test | bench | clean>
cd "$(dirname "$0")" || exit 1
for t in cargo clang; do
  command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t (need Rust + LLVM clang/lld)"; exit 1; }
done
# Wipe stale build outputs once whenever VERSION changes (zip timestamps can fool cargo).
ver=$(tr -d '[:space:]' < VERSION)
if [ "$(cat target/.rvsim_version 2>/dev/null)" != "$ver" ]; then
  echo "rvsim $ver : removing stale build files from an older version (one-time)..."
  rm -rf target build
  mkdir -p target && echo "$ver" > target/.rvsim_version
fi
exec cargo run --release -q -p xtask -- "${@:-help}"
