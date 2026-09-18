export PATH="$HOME/local/.cargo/bin:/usr/bin:$PATH"
export RUSTUP_HOME="$HOME/local/.rustup"
export VITRI_CXX=g++-12
export CARGO_TERM_COLOR=never
set -o pipefail
touch src/lib.rs
echo "### check"
cargo check --all-targets --locked 2>&1 | tee ab/check.txt | grep -cE "^error" || true
echo "### ERRORS BEGIN"
grep -E "^(error|warning: unused)" -A 4 ab/check.txt | head -400
echo "### ERRORS END"
echo "### DONE"
