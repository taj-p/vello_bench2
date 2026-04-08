#!/usr/bin/env bash
set -euo pipefail

DIST=dist
TARGET=wasm32-unknown-unknown

build_variant() {
  local rustflags=$1
  local out_dir=$2

  echo "==> Building $out_dir..."
  RUSTFLAGS="$rustflags" cargo build --target $TARGET --profile instrument

  echo "==> Running wasm-bindgen ($out_dir)..."
  mkdir -p "$DIST/$out_dir"
  wasm-bindgen \
    --target web \
    --out-dir "$DIST/$out_dir" \
    --no-typescript \
    target/$TARGET/instrument/vello_bench2.wasm
}

# Parse arguments.
FILTER="all"
BIND_ADDR="127.0.0.1"

for arg in "$@"; do
  case "$arg" in
    --global) BIND_ADDR="0.0.0.0" ;;
    *) FILTER="$arg" ;;
  esac
done

should_build() {
  local out_dir=$1
  [[ "$FILTER" == "all" ]] && return 0
  # Match if the filter is a substring of the variant name.
  [[ "$out_dir" == *"$FILTER"* ]] && return 0
  return 1
}

# Remove stale A/B directories so index.html doesn't detect ab.sh mode.
rm -rf "$DIST/control" "$DIST/treatment"

should_build simd   && build_variant "-Ctarget-feature=+simd128" simd
should_build nosimd && build_variant ""                          nosimd

cp web/index.html "$DIST/index.html"

# ── Serve ─────────────────────────────────────────────────────────────────────

echo "==> Serving at http://localhost:8080"
if [[ "$BIND_ADDR" == "0.0.0.0" ]]; then
  LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || echo "<your-ip>")
  echo "==> On your tablet, open http://$LOCAL_IP:8080"
fi
python3 -c "
import http.server, os

os.chdir('$DIST')

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.ThreadingHTTPServer(('$BIND_ADDR', 8080), Handler).serve_forever()
"
