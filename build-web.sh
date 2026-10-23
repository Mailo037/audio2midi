#!/usr/bin/env bash
# Baut die Browser-Version nach docs/ (wird von GitHub Pages ausgeliefert).
# Voraussetzungen: rustup target add wasm32-unknown-unknown
#                  cargo install wasm-bindgen-cli --version <Version aus Cargo.lock>
set -euo pipefail
cd "$(dirname "$0")"
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+simd128" \
  cargo build --release -p audio2midi-app --target wasm32-unknown-unknown
rm -rf docs/pkg
wasm-bindgen --target web --no-typescript --out-dir docs/pkg \
  target/wasm32-unknown-unknown/release/audio2midi-app.wasm
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int \
    docs/pkg/audio2midi-app_bg.wasm -o docs/pkg/audio2midi-app_bg.wasm
fi
cp web/index.html docs/index.html
touch docs/.nojekyll
echo "Fertig: docs/ ($(du -sh docs/pkg | cut -f1))"
