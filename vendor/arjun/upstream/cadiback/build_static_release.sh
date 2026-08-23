#!/bin/bash
set -e
rm -rf .cmake
rm -rf lib*
rm -rf Test*
rm -rf tests*
rm -rf include
rm -rf tests
rm -rf approxmc*
rm -rf apx-src
rm -rf CM*
rm -rf cmake*
rm -rf deps
rm -rf _deps
SAT_DIR=$(cd ../.. && pwd)
cmake -S .. -B . -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
  -Dcadical_DIR=${SAT_DIR}/cadical/build \
  -DCMAKE_INSTALL_PREFIX="$(pwd)"
cmake --build . -j$(nproc)
cmake --install .
