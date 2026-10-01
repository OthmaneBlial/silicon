#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo build -p silicon-c-api
mkdir -p output target
compiler=${CC:-cc}
"$compiler" -std=c11 -Wall -Wextra -Werror \
  -Icrates/silicon-c-api/include \
  crates/silicon-c-api/examples/triangle.c \
  -Ltarget/debug -lsilicon_c_api \
  -Wl,-rpath,"$repo_root/target/debug" \
  -o target/c_api_triangle
target/c_api_triangle
