//! Build script: compiles the vendored Arjun preprocessing stack, five CMake
//! projects under
//!   `vendor/arjun/upstream/`. `build.rs` drives CMake itself, so a plain
//!   `cargo build` produces a working Arjun with no install script, no network
//!   access and no out-of-tree state. See `vendor/arjun/upstream/PROVENANCE.md`.
//!
//! Everything a published crate needs is inside the package: `cargo build` from
//! a freshly unpacked `.crate` works with the network unavailable, because the
//! vendored sources are complete and CMake runs with
//! `FETCHCONTENT_FULLY_DISCONNECTED=ON`.
//!
//! A build for Emscripten compiles the same stack with the Emscripten SDK;
//! [`toolchain::Toolchain::emscripten`] says what else it needs.

#[path = "build/cmake.rs"]
mod cmake;
#[path = "build/link.rs"]
mod link;
#[path = "build/prereqs.rs"]
mod prereqs;
#[path = "build/run.rs"]
mod run;
#[path = "build/toolchain.rs"]
mod toolchain;

use std::path::{Path, PathBuf};

/// The optimisation level every vendored C++ translation unit is compiled at.
const CXX_OPT_LEVEL: u32 = 3;

fn main() {
    // docs.rs builds documentation, not a binary: rustdoc type-checks the crate
    // and never links it, so the `extern "C"` declarations resolve to nothing and
    // no native code is needed. That matters because docs.rs cannot provide CMake
    // or GMP/MPFR, so the vendored stack cannot be built in its sandbox — and the
    // stack is not optional. Skipping just the native build documents the whole
    // crate: `preprocess` (Arjun, every counting mode, the lift
    // record) renders exactly as a normal `cargo add vitri` build sees it.
    //
    // `DOCS_RS` is set by docs.rs itself; a normal build never takes this path.
    println!("cargo:rerun-if-env-changed=DOCS_RS");
    if std::env::var_os("DOCS_RS").is_some() {
        return;
    }

    // One compiler choice and one prerequisite check for the Arjun build.
    println!("cargo:rerun-if-env-changed=VITRI_CXX");
    println!(
        "cargo:rerun-if-env-changed={}",
        toolchain::EMSCRIPTEN_PREFIX
    );
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    // Set by cargo for build scripts; the target's OS, not the host's.
    let toolchain = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten") {
        prereqs::require_prereqs(&out_dir, None);
        toolchain::Toolchain::emscripten()
    } else {
        let (cc, cxx) = toolchain::find_cxx();
        prereqs::require_prereqs(&out_dir, Some((&cc, &cxx)));
        toolchain::Toolchain::native(cc, cxx)
    };

    build_arjun(&out_dir, &toolchain);
}

// ------------------------------------------------- vendored C++ SAT stack
//
// One CMake build produces everything downstream of it: Arjun (plus
// CryptoMiniSat and CadiBack) requires meelgroup's CaDiCaL fork, and our own
// preprocessing links that same fork through `cadical_shim` instead of a
// second, stock copy — so there is exactly one CaDiCaL in the process.

fn build_arjun(out_dir: &Path, toolchain: &toolchain::Toolchain) {
    println!("cargo:rerun-if-changed=vendor/arjun/");

    let libs = cmake::build_vendored(out_dir, toolchain);

    link::link_shim(out_dir, toolchain, &libs);
}
