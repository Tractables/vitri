//! The one CMake build: Arjun, CryptoMiniSat, CadiBack and the CaDiCaL fork
//! they all link, configured and built into `OUT_DIR`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CXX_OPT_LEVEL;
use crate::run::{arjun_pin, run};
use crate::toolchain::{Libs, Toolchain};

/// Configure and build the vendored CMake projects into `OUT_DIR`.
///
/// Three properties matter and are all enforced here, because a published
/// crate gets none of them for free:
/// * **offline** — `FETCHCONTENT_FULLY_DISCONNECTED=ON` plus an explicit
///   `FETCHCONTENT_SOURCE_DIR_<NAME>` per dependency. Upstream would clone
///   `GIT_TAG master`; with this, a missing override fails loudly instead of
///   quietly building something we never pinned.
/// * **out-of-source** — cargo gives a build script exactly one writable
///   directory, `OUT_DIR`. The crate source may be read-only.
/// * **MPL2-only Eigen** — SBVA bundles Eigen, which is MPL-2.0 with some
///   LGPL files. `EIGEN_MPL2_ONLY` turns including an LGPL header into a
///   compile error, so the licence property is enforced by the build.
pub(crate) fn build_vendored(out_dir: &Path, toolchain: &Toolchain) -> Libs {
    let vendor =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("vendor/arjun/upstream");
    assert!(
        vendor.join("arjun/CMakeLists.txt").exists(),
        "vendored Arjun sources missing at {} — the package is incomplete \
         (check the `include` allowlist in Cargo.toml).",
        vendor.display()
    );

    let build_dir = out_dir.join("arjun-build");
    // A CMake build directory is bound to the source directory that first
    // configured it: aim the same build dir at a different source and CMake
    // refuses with "does not match the source used to generate cache".
    // Cargo can hand us an OUT_DIR a previous build already configured from
    // a DIFFERENT path — `cargo package`/`cargo publish` build this very
    // crate from `target/package/<pkg>/` while reusing the target directory
    // they were invoked in. So a plain `cargo build` followed by
    // `cargo publish` trips a stale cache through no fault of the user, and
    // the error names CMake rather than the cause.
    //
    // Discard a build dir whose cache came from another source. It is pure
    // build output: dropping it costs a rebuild and nothing else.
    let cache = build_dir.join("CMakeCache.txt");
    if let Ok(text) = std::fs::read_to_string(&cache) {
        let want = vendor.join("arjun");
        let stale = !text
            .lines()
            .filter_map(|l| l.strip_prefix("CMAKE_HOME_DIRECTORY:INTERNAL="))
            .any(|home| Path::new(home.trim()) == want);
        if stale {
            let _ = std::fs::remove_dir_all(&build_dir);
        }
    }
    std::fs::create_dir_all(&build_dir).expect("create arjun build dir");

    let src = |name: &str| {
        let p = vendor.join(name);
        assert!(p.exists(), "vendored dependency missing: {}", p.display());
        p
    };

    let (program, before) = toolchain
        .cmake
        .split_first()
        .expect("a toolchain names the command that runs CMake");
    let mut cfg = Command::new(program);
    cfg.args(before)
        .arg("-S")
        .arg(vendor.join("arjun"))
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .args(&toolchain.cmake_defines)
        // Static: the shim is linked into one shared object below, and
        // nothing else may resolve these symbols.
        .arg("-DBUILD_SHARED_LIBS=OFF")
        .arg("-DENABLE_TESTING=OFF")
        .arg("-DFETCHCONTENT_FULLY_DISCONNECTED=ON");
    // Each dependency Arjun's CMake would otherwise fetch, pointed at the
    // vendored tree instead. The name on the left is CMake's, the one on
    // the right is the directory's.
    for (project, dir) in [
        ("CADICAL", "cadical"),
        ("CRYPTOMINISAT5", "cryptominisat"),
        ("CADIBACK", "cadiback"),
        ("SBVA", "sbva"),
    ] {
        cfg.arg(format!(
            "-DFETCHCONTENT_SOURCE_DIR_{project}={}",
            src(dir).display()
        ));
    }
    // `Release` already implies `-O3`; naming the level here is what makes
    // `CXX_OPT_LEVEL` the one place it is decided, so moving it moves both
    // halves of the build together.
    cfg.arg(format!(
        "-DCMAKE_CXX_FLAGS=-DEIGEN_MPL2_ONLY -O{CXX_OPT_LEVEL} {}",
        toolchain.cxx_flags.join(" ")
    ))
    // The vendored tree has no .git, so Arjun's own git probe would bake
    // an EMPTY "Arjun SHA1:" into the binary — the identity every
    // consumer checks to spot a stale or foreign install. Pass the pin
    // explicitly; `arjun/CMakeLists.txt` was modified to honour it.
    .arg(format!("-DGIT_SHA1={}", arjun_pin(&vendor)));
    run(cfg, "cmake configure (Arjun stack)");

    // The libraries merged below, not the projects' command-line programs,
    // which vitri does not use. `oracle` is not a dependency of `arjun`.
    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .args(["--target", "arjun", "oracle"]);
    if let Ok(jobs) = std::env::var("NUM_JOBS") {
        build.arg("-j").arg(jobs);
    }
    run(build, "cmake build (Arjun stack)");

    // Layout produced by the projects above, in dependency order —
    // `--start-group` in `link_shim` makes the arjun <-> cms <-> cadical
    // cycle resolvable regardless. Asserted rather than globbed: a
    // silently-missing archive would link, then fail at the first Arjun
    // call with an undefined symbol.
    let deps = build_dir.join("_deps");
    let archives = vec![
        build_dir.join("lib/libarjun.a"),
        deps.join("sbva-build/lib/libsbva.a"),
        deps.join("cryptominisat5-build/lib/libcryptominisat5.a"),
        deps.join("cryptominisat5-build/lib/liboracle.a"),
        deps.join("cadiback-build/libcadiback.a"),
        deps.join("cadical-build/libcadical.a"),
    ];
    for a in &archives {
        assert!(
            a.exists(),
            "Arjun build produced no {} — build layout changed?",
            a.display()
        );
    }

    Libs {
        includes: vec![
            vendor.join("arjun/src"),
            // `cadical.hpp` for cadical_shim.cpp. Same tree the archive
            // above was built from, so the shim cannot drift from the
            // CaDiCaL it calls into.
            vendor.join("cadical/src"),
            // CryptoMiniSat's public headers are generated into the build
            // tree (as links into its source), so this path only exists
            // after the build above.
            deps.join("cryptominisat5-build/include"),
        ]
        .into_iter()
        .chain(toolchain.includes.iter().cloned())
        .collect(),
        archives,
    }
}
