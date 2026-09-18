export PATH="$HOME/local/.cargo/bin:/usr/bin:$PATH"
export RUSTUP_HOME="$HOME/local/.rustup"
export VITRI_CXX=g++-12
export CARGO_TERM_COLOR=never

DELETED=""
for f in $DELETED; do rm -f "$f"; done

set -o pipefail
fail() { echo "### FAIL: $1"; exit 1; }

touch src/lib.rs

echo "### fmt"
cargo fmt --check > ab/fmt.txt 2>&1 || {
  echo "### FMT DIFF BEGIN"; cat ab/fmt.txt; echo "### FMT DIFF END"; fail fmt; }

echo "### example lock"
cargo metadata --manifest-path examples/rsdd-count/Cargo.toml --format-version 1 > /dev/null || fail example-lock

echo "### build"
cargo build --release --locked 2>&1 | tail -40 || fail build

echo "### clippy"
cargo clippy --release --all-targets --locked -- -D warnings 2>&1 | tail -40 || fail clippy

echo "### test"
cargo test --release --all-targets --locked 2>&1 | grep -v '^test .* ok$' | tail -60 || fail test

echo "### ignored"
cargo test --release --all-targets --locked -- --ignored 2>&1 | tail -8 || fail ignored

echo "### doctest"
cargo test --release --doc --locked 2>&1 | tail -8 || fail doctest

echo "### doc"
DOCS_RS=1 RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked 2>&1 | tail -8 || fail doc

echo "### example"
cargo clippy --release --locked --manifest-path examples/rsdd-count/Cargo.toml -- -D warnings 2>&1 | tail -20 || fail example

echo "### package"
cargo package --locked --allow-dirty 2>&1 | tail -5 || fail package

echo "### packaged tests"
( cd target/package/vitri-*/ && cargo test --release --locked --all-targets 2>&1 | grep -v '^test .* ok$' | tail -20 ) || fail packaged

echo "### c"
( cd bindings/c && make libs && make link-libs-check && make check && make linkage \
  && cargo test --release --locked 2>&1 | tail -5 && cargo fmt --check \
  && cargo clippy --release --all-targets --locked -- -D warnings 2>&1 | tail -10 ) || fail c

echo "### python"
( cd bindings/python && cargo fmt --check \
  && PYO3_PYTHON=$(command -v python3) cargo clippy --release --locked -- -D warnings 2>&1 | tail -20 ) || fail python

echo "### OK"
