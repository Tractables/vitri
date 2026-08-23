#!/bin/bash
set -e
cmake -S .. -B . -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DCMAKE_INSTALL_PREFIX="$(pwd)"
cmake --build . -j$(nproc)
cmake --install .
