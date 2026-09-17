//! What the build does differently per target: how CMake is invoked, which
//! compiler and archiver compile and merge the translation units outside it,
//! and where GMP and MPFR come from.
//!
//! One value carries all of it; the steps that use it do not test the target
//! again.

use std::path::PathBuf;

use crate::run::have;

/// Where the Arjun archives and headers the shim links against live.
pub(crate) struct Libs {
    /// Directories to pass as `-I` when compiling the shim.
    pub(crate) includes: Vec<PathBuf>,
    /// Static archives, in link order.
    pub(crate) archives: Vec<PathBuf>,
}

/// The environment variable naming the prefix that holds GMP and MPFR
/// built for Emscripten.
pub(crate) const EMSCRIPTEN_PREFIX: &str = "VITRI_EMSCRIPTEN_PREFIX";

/// Everything about the build that depends on the target: how CMake is run
/// and configured, the compiler and archiver for the translation units
/// compiled outside CMake, and what the merged archive is linked with.
pub(crate) struct Toolchain {
    /// The command that runs CMake, and the arguments before CMake's own.
    pub(crate) cmake: &'static [&'static str],
    /// Configure settings beyond the ones every build passes.
    pub(crate) cmake_defines: Vec<String>,
    pub(crate) cxx: String,
    /// Flags every C++ translation unit takes on this target, inside CMake
    /// and out.
    pub(crate) cxx_flags: &'static [&'static str],
    pub(crate) ar: String,
    /// Header directories the translation units compiled outside CMake need
    /// beyond the stack's own, such as GMP's when it is not the system's.
    pub(crate) includes: Vec<PathBuf>,
    /// vitri's own translation units under `vendor/arjun/`.
    pub(crate) shims: &'static [&'static str],
    /// Directories added to the linker's search path: where `dylibs` are,
    /// or where emcc looks for the libraries a side module needs.
    pub(crate) dylib_dirs: Vec<PathBuf>,
    /// Libraries linked dynamically by name, for the reason
    /// `links_system_libs` gives. There is deliberately no switch.
    pub(crate) dylibs: &'static [&'static str],
    /// Side modules the program has to name on its own link line, where
    /// libraries linked dynamically by name are not enough.
    pub(crate) side_modules: Vec<PathBuf>,
}

impl Toolchain {
    /// A native build: `cc` and `cxx` as [`find_cxx`] chose them, `AR` or
    /// else `ar`, and GMP, MPFR, zlib and the C++ runtime from the system.
    pub(crate) fn native(cc: String, cxx: String) -> Self {
        if std::env::var_os(EMSCRIPTEN_PREFIX).is_some() {
            panic!(
                "{EMSCRIPTEN_PREFIX} names the GMP and MPFR a build for Emscripten links, \
                 and has no effect on a native build, which uses the system's. Unset it, \
                 or build for wasm32-unknown-emscripten."
            );
        }
        println!("cargo:rerun-if-env-changed=AR");
        Toolchain {
            cmake: &["cmake"],
            cmake_defines: vec![
                format!("-DCMAKE_C_COMPILER={cc}"),
                format!("-DCMAKE_CXX_COMPILER={cxx}"),
            ],
            cxx,
            cxx_flags: &[],
            ar: std::env::var("AR").unwrap_or_else(|_| "ar".to_string()),
            includes: Vec::new(),
            // arjun_shim exposes Arjun itself; cadical_shim backs our own
            // preprocessing.
            shims: &["cadical_shim.cpp", "arjun_shim.cpp"],
            dylib_dirs: Vec::new(),
            dylibs: &["stdc++", "gmpxx", "gmp", "mpfr", "z"],
            side_modules: Vec::new(),
        }
    }

    /// A build for Emscripten: the SDK's `emcmake`, `em++` and `emar`, found
    /// on `PATH`, and GMP from the prefix named by `VITRI_EMSCRIPTEN_PREFIX`.
    ///
    /// That prefix holds GMP (with its C++ interface) and MPFR built for
    /// Emscripten and installed there, plus GMP as the side modules
    /// `lib/libgmp.so` and `lib/libgmpxx.so`, which the program links
    /// dynamically and loads at run time. A program linked with
    /// `-sMAIN_MODULE` takes a side module only from a path on its link
    /// line, not from `-l`, and a build script's link arguments do not reach
    /// the crates that depend on this one, so the two paths are published
    /// as the `side_modules` metadata instead, which a dependent's build
    /// script reads as `DEP_VITRI_ARJUN_SIDE_MODULES`. Arjun's CMake needs
    /// MPFR to configure; nothing vitri links calls it, so no MPFR side
    /// module is needed. Nothing links zlib either, so the stack is
    /// configured without it.
    ///
    /// Everything is compiled with `-fwasm-exceptions`, because rustc links
    /// this target with WebAssembly exception handling, and with `-fPIC`,
    /// because a program that loads side modules is linked from
    /// position-independent code.
    ///
    /// libc's `getrusage` is replaced by `emscripten_getrusage.cpp`, which
    /// says why.
    pub(crate) fn emscripten() -> Self {
        if std::env::var_os("VITRI_CXX").is_some_and(|cxx| !cxx.is_empty()) {
            panic!(
                "VITRI_CXX chooses the compiler of a native build, and has no effect on \
                 a build for Emscripten, which compiles with the SDK's em++. Unset it for \
                 this target."
            );
        }
        let prefix = std::env::var_os(EMSCRIPTEN_PREFIX)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!(
                    "building vitri for Emscripten needs {EMSCRIPTEN_PREFIX}: the prefix \
                     holding GMP and MPFR built for Emscripten, with GMP's side modules \
                     (docs/building.md)"
                )
            });
        let lib = prefix.join("lib");
        for needed in [
            "include/gmp.h",
            "include/gmpxx.h",
            "lib/pkgconfig/gmp.pc",
            "lib/pkgconfig/gmpxx.pc",
            "lib/pkgconfig/mpfr.pc",
            "lib/libgmp.so",
            "lib/libgmpxx.so",
        ] {
            let path = prefix.join(needed);
            assert!(
                path.is_file(),
                "{EMSCRIPTEN_PREFIX}={} has no {needed}: build GMP and MPFR for \
                 Emscripten into it, and GMP's side modules (docs/building.md)",
                prefix.display()
            );
            println!("cargo:rerun-if-changed={}", path.display());
        }
        Toolchain {
            cmake: &["emcmake", "cmake"],
            cmake_defines: vec![
                format!("-DCMAKE_PREFIX_PATH={}", prefix.display()),
                "-DCMAKE_C_FLAGS=-fPIC".to_string(),
                "-DNOZLIB=ON".to_string(),
            ],
            cxx: "em++".to_string(),
            cxx_flags: &["-fwasm-exceptions", "-fPIC"],
            ar: "emar".to_string(),
            includes: vec![prefix.join("include")],
            shims: &[
                "cadical_shim.cpp",
                "arjun_shim.cpp",
                "emscripten_getrusage.cpp",
            ],
            // libgmpxx.so needs libgmp.so, and emcc finds it here.
            dylib_dirs: vec![lib.clone()],
            // Emscripten links its own C++ runtime into the program.
            dylibs: &[],
            side_modules: vec![lib.join("libgmpxx.so"), lib.join("libgmp.so")],
        }
    }
}

/// Arjun's C++20 (`constexpr std::vector` copies) needs gcc-12 or newer;
/// Ubuntu 22.04 still ships gcc-11 as `g++`. Prefer an explicit `VITRI_CXX`,
/// else the newest versioned gcc on PATH, else plain `g++` — which may well
/// be new enough on a current distro.
///
/// Choosing is all this does; whether the choice can build anything is
/// [`require_prereqs`].
pub(crate) fn find_cxx() -> (String, String) {
    if let Ok(cxx) = std::env::var("VITRI_CXX")
        && !cxx.is_empty()
    {
        let cc = cxx.replace("g++", "gcc").replace("clang++", "clang");
        return (cc, cxx);
    }
    for v in ["14", "13", "12"] {
        if have(&format!("g++-{v}")) && have(&format!("gcc-{v}")) {
            return (format!("gcc-{v}"), format!("g++-{v}"));
        }
    }
    ("gcc".into(), "g++".into())
}
