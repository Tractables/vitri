#!/bin/bash
set -e
SAT_DIR=$(cd .. && pwd)
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
  -Dcadical_DIR=${SAT_DIR}/cadical/build \
  -DCMAKE_INSTALL_PREFIX="$(pwd)"
cmake --build build -j$(nproc)
cmake --install build
