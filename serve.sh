#!/usr/bin/env bash
set -euo pipefail

DIST=dist
TARGET=wasm32-unknown-unknown

build_variant() {
  local features=$1
  local rustflags=$2
  local out_dir=$3

  echo "==> Building $out_dir..."
  RUSTFLAGS="$rustflags" cargo build --target $TARGET --release --no-default-features --features "$features"

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    target/$TARGET/release/vello_bench2.wasm
}

build_variant hybrid "-Ctarget-feature=+simd128" hybrid-simd
build_variant hybrid ""                          hybrid-nosimd
build_variant cpu    "-Ctarget-feature=+simd128" cpu-simd
build_variant cpu    ""                          cpu-nosimd

cp web/index.html "$DIST/index.html"

echo "==> Serving at http://localhost:8080"
python3 -m http.server 8080 --directory "$DIST"
