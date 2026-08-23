#!/usr/bin/env bash

set -euo pipefail

SAT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
echo "solvers dir: $SAT_DIR"
rm -rf cm*
rm -rf CM*
rm -rf lib*
rm -rf Testing*
rm -rf tests*
rm -rf include
rm -rf tests
rm -rf utils
rm -rf Make*
rm -rf deps
rm -rf _deps
emcmake cmake -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DCMAKE_INSTALL_PREFIX="${EMINSTALL}"  \
  -DENABLE_TESTING=OFF ..
emmake make -j$(nproc)
emmake make install
# cp cadiback.wasm "${SAT_DIR}/cadiback/html"
# cp "${EMINSTALL}/bin/cadiback.js" "${SAT_DIR}/cadiback/html"
