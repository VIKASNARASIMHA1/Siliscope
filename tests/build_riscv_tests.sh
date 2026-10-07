#!/bin/bash
# Rebuilds the prebuilt tests in tests/riscv-tests/ from the upstream sources.
#   git clone --recurse-submodules https://github.com/riscv-software-src/riscv-tests
#   cd riscv-tests && bash /path/to/rvsim/tests/build_riscv_tests.sh
# Needs clang + lld. The sed step works around a clang assembler quirk
# (".weak" followed by ".global" on the same symbol).
set -e
OUT=${OUT:-out}; mkdir -p "$OUT"
CF="--target=riscv64-unknown-elf -march=rv64imac_zicsr_zifencei -mabi=lp64 -mcmodel=medany -mno-relax"
for suite in rv64ui rv64um rv64ua rv64uc rv64mi rv64si; do
  for f in isa/$suite/*.S; do
    n=$(basename "$f" .S)
    clang $CF -E -x assembler-with-cpp -Ienv/p -Iisa/macros/scalar "$f" 2>/dev/null \
      | sed -E 's/\.globl?[[:space:]]+(m|s)tvec_handler/.weak \1tvec_handler/' > "$OUT/$n.s"
    clang $CF -c "$OUT/$n.s" -o "$OUT/$n.o" 2>/dev/null || { echo "skip $suite/$n (needs unsupported extension)"; continue; }
    clang $CF -static -nostdlib -fuse-ld=lld -Wl,-T,env/p/link.ld "$OUT/$n.o" -o "$OUT/$suite-p-$n"
  done
done
