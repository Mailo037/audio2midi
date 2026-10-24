#!/usr/bin/env bash
# Baut die Browser-Version nach site/ (GitHub Actions veröffentlicht sie auf GitHub Pages).
# Voraussetzungen: rustup target add wasm32-unknown-unknown
#                  cargo install wasm-bindgen-cli --version <Version aus Cargo.lock>
set -euo pipefail
cd "$(dirname "$0")"
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+simd128" \
  cargo build --release -p audio2midi-app --target wasm32-unknown-unknown
rm -rf site/pkg
wasm-bindgen --target web --no-typescript --out-dir site/pkg \
  target/wasm32-unknown-unknown/release/audio2midi-app.wasm
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int \
    site/pkg/audio2midi-app_bg.wasm -o site/pkg/audio2midi-app_bg.wasm
fi
cp web/index.html site/index.html
touch site/.nojekyll
echo "Done: site/ ($(du -sh site/pkg | cut -f1))"
