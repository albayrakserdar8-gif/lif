#!/bin/sh
# zig as the linker when cross-compiling lif for aarch64 Linux from macOS:
#   CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$PWD/tools/zigcc-aarch64-linux.sh \
#     cargo build -p lif --target aarch64-unknown-linux-gnu
# rustc passes one aarch64 erratum flag zig's linker does not know; it is
# dropped (the workaround matters only on Cortex-A53 hardware).
n=$#
i=0
while [ $i -lt $n ]; do
    a=$1
    shift
    i=$((i + 1))
    case "$a" in
    -Wl,--fix-cortex-a53-843419) ;;
    *) set -- "$@" "$a" ;;
    esac
done
exec zig cc -target aarch64-linux-gnu "$@"
