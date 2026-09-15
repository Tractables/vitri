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
//! [`arjun::Toolchain::emscripten`] says what else it needs.

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
    println!("cargo:rerun-if-env-changed={}", arjun::EMSCRIPTEN_PREFIX);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    // Set by cargo for build scripts; the target's OS, not the host's.
    let toolchain = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten") {
        arjun::Toolchain::emscripten()
    } else {
        let (cc, cxx) = arjun::find_cxx();
        arjun::require_prereqs(&out_dir, &cxx);
        arjun::Toolchain::native(cc, cxx)
    };

    build_arjun(&out_dir, &toolchain);
}

// ------------------------------------------------- vendored C++ SAT stack
//
// One CMake build produces everything downstream of it: Arjun (plus
// CryptoMiniSat and CadiBack) requires meelgroup's CaDiCaL fork, and our own
// preprocessing links that same fork through `cadical_shim` instead of a
// second, stock copy — so there is exactly one CaDiCaL in the process.

fn build_arjun(out_dir: &Path, toolchain: &arjun::Toolchain) {
    println!("cargo:rerun-if-changed=vendor/arjun/");

    let libs = arjun::build_vendored(out_dir, toolchain);

    arjun::link_shim(out_dir, toolchain, &libs);
}

mod arjun {
    use super::CXX_OPT_LEVEL;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Where the Arjun archives and headers the shim links against live.
    pub struct Libs {
        /// Directories to pass as `-I` when compiling the shim.
        pub includes: Vec<PathBuf>,
        /// Static archives, in link order.
        pub archives: Vec<PathBuf>,
    }

    /// The environment variable naming the prefix that holds GMP and MPFR
    /// built for Emscripten.
    pub const EMSCRIPTEN_PREFIX: &str = "VITRI_EMSCRIPTEN_PREFIX";

    /// Everything about the build that depends on the target: how CMake is run
    /// and configured, the compiler and archiver for the translation units
    /// compiled outside CMake, and what the merged archive is linked with.
    pub struct Toolchain {
        /// The command that runs CMake, and the arguments before CMake's own.
        cmake: &'static [&'static str],
        /// Configure settings beyond the ones every build passes.
        cmake_defines: Vec<String>,
        cxx: String,
        /// Flags every C++ translation unit takes on this target, inside CMake
        /// and out.
        cxx_flags: &'static [&'static str],
        ar: String,
        /// Header directories the translation units compiled outside CMake need
        /// beyond the stack's own, such as GMP's when it is not the system's.
        includes: Vec<PathBuf>,
        /// vitri's own translation units under `vendor/arjun/`.
        shims: &'static [&'static str],
        /// Directories added to the linker's search path: where `dylibs` are,
        /// or where emcc looks for the libraries a side module needs.
        dylib_dirs: Vec<PathBuf>,
        /// Libraries linked dynamically by name, for the reason
        /// `links_system_libs` gives. There is deliberately no switch.
        dylibs: &'static [&'static str],
        /// Side modules the program has to name on its own link line, where
        /// libraries linked dynamically by name are not enough.
        side_modules: Vec<PathBuf>,
    }

    impl Toolchain {
        /// A native build: `cc` and `cxx` as [`find_cxx`] chose them, `AR` or
        /// else `ar`, and GMP, MPFR, zlib and the C++ runtime from the system.
        pub fn native(cc: String, cxx: String) -> Self {
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
        pub fn emscripten() -> Self {
            if std::env::var_os("VITRI_CXX").is_some_and(|cxx| !cxx.is_empty()) {
                panic!(
                    "VITRI_CXX chooses the compiler of a native build, and has no effect on \
                     a build for Emscripten, which compiles with the SDK's em++. Unset it for \
                     this target."
                );
            }
            // emcmake sits beside em++ and has no --version to probe.
            for tool in ["em++", "emar", "cmake", "pkg-config"] {
                assert!(
                    have(tool),
                    "building vitri for Emscripten needs `{tool}` on PATH: install and \
                     activate the Emscripten SDK, source its emsdk_env.sh, and install \
                     CMake and pkg-config (docs/building.md)"
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
    pub fn find_cxx() -> (String, String) {
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

    fn have(tool: &str) -> bool {
        Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// How the build looks for one prerequisite.
    enum Probe {
        /// The named program answers `--version`.
        OnPath(&'static str),
        /// The chosen C++ compiler answers `--version`. Which program that is
        /// comes from [`find_cxx`], not from the table.
        Compiler,
        /// A one-file program using all three libraries compiles and links.
        Links,
    }

    /// One prerequisite of the vendored C++ build: how the build looks for it,
    /// and the package that carries it in each package manager the failure
    /// message offers — empty where the platform ships it outside one.
    struct Prereq {
        /// What the build looks for, worded as the message names it.
        what: &'static str,
        probe: Probe,
        apt: &'static str,
        dnf: &'static str,
        brew: &'static str,
    }

    /// THE prerequisite list: [`require_prereqs`] checks these in order, every
    /// failure message prints install commands built from them, and
    /// `docs/building.md` publishes those same commands.
    const PREREQS: &[Prereq] = &[
        Prereq {
            what: "a C++20 compiler (gcc 12 or newer)",
            probe: Probe::Compiler,
            apt: "build-essential gcc-12 g++-12",
            dnf: "gcc-c++",
            // Apple ships the toolchain with the Xcode command line tools.
            brew: "",
        },
        Prereq {
            what: "CMake",
            probe: Probe::OnPath("cmake"),
            apt: "cmake",
            dnf: "cmake",
            brew: "cmake",
        },
        Prereq {
            what: "pkg-config",
            probe: Probe::OnPath("pkg-config"),
            apt: "pkg-config",
            dnf: "pkgconf-pkg-config",
            brew: "pkg-config",
        },
        Prereq {
            what: "the GMP, MPFR and zlib development packages",
            probe: Probe::Links,
            apt: "libgmp-dev libmpfr-dev zlib1g-dev",
            dnf: "gmp-devel mpfr-devel zlib-devel",
            brew: "gmp mpfr zlib",
        },
    ];

    /// Check every prerequisite, before either half of the build starts.
    ///
    /// Unconditional, because none of the ways [`find_cxx`] can arrive at a
    /// compiler implies that CMake, pkg-config or the system libraries are
    /// installed. Three `--version` runs and one small compile buy the
    /// difference between a sentence naming the missing package and a CMake
    /// configure error, or a wall of linker noise minutes into the build.
    pub fn require_prereqs(out_dir: &Path, cxx: &str) {
        for prereq in PREREQS {
            let wrong = match prereq.probe {
                Probe::Compiler => (!have(cxx)).then(|| {
                    format!("`{cxx}` does not run — install one, or name another in VITRI_CXX")
                }),
                Probe::OnPath(tool) => (!have(tool)).then(|| format!("`{tool}` is not on PATH")),
                Probe::Links => (!links_system_libs(out_dir, cxx))
                    .then(|| format!("at least one is missing or unusable with `{cxx}`")),
            };
            if let Some(detail) = wrong {
                panic!(
                    "vitri's vendored C++ stack needs {}, and {detail}.\n\
                     Install every prerequisite with one of:\n{}\n\
                     docs/building.md says what each one is for.",
                    prereq.what,
                    install_commands()
                );
            }
        }
        warn_if_doc_drifted();
    }

    /// One install command per package manager, each covering EVERY
    /// prerequisite: a machine missing one is usually missing more, and a
    /// command that ends the problem beats four that each end a quarter of it.
    fn install_commands() -> String {
        let packages = |pick: fn(&Prereq) -> &'static str| {
            PREREQS
                .iter()
                .map(pick)
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let commands = [
            (
                format!("sudo apt install {}", packages(|p| p.apt)),
                "Debian/Ubuntu",
            ),
            (
                format!("sudo dnf install {}", packages(|p| p.dnf)),
                "Fedora/RHEL",
            ),
            (format!("brew install {}", packages(|p| p.brew)), "macOS"),
        ];
        let width = commands.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
        commands
            .iter()
            .map(|(c, platform)| format!("  {c:width$}   # {platform}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// GMP, MPFR and zlib are system packages — deliberately NOT vendored. Both
    /// GMP and MPFR are LGPL, so folding them in statically would attach LGPL
    /// relinking obligations to every binary built from this Apache-2.0 crate.
    /// They are therefore always taken from the system, which is why their
    /// absence has to be a build failure rather than a fallback.
    fn links_system_libs(out_dir: &Path, cxx: &str) -> bool {
        let probe = out_dir.join("probe_system_libs.cpp");
        std::fs::write(
            &probe,
            "#include <gmpxx.h>\n#include <mpfr.h>\n#include <zlib.h>\n\
             int main(){ mpz_class z(1); mpfr_t f; mpfr_init(f); mpfr_clear(f); \
             (void)zlibVersion(); return z.get_si()-1; }\n",
        )
        .expect("write system-lib probe");

        Command::new(cxx)
            .args(["-std=c++20", "-o"])
            .arg(out_dir.join("probe_system_libs"))
            .arg(&probe)
            .args(["-lgmpxx", "-lgmp", "-lmpfr", "-lz"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// The install commands in `docs/building.md` are the ones printed above;
    /// say so out loud when the two have drifted apart.
    ///
    /// A warning and not a failure: a reader's build must not stop over
    /// documentation. The doc ships inside the package, so a consumer runs this
    /// too, and it is silent unless the two really disagree.
    fn warn_if_doc_drifted() {
        println!("cargo:rerun-if-changed=docs/building.md");
        // cargo runs a build script with the package root as its working
        // directory, which is why every path here is relative to it.
        let Ok(published) = std::fs::read_to_string("docs/building.md") else {
            return;
        };
        for command in install_commands().lines() {
            let command = command.trim();
            if !published.contains(command) {
                println!(
                    "cargo:warning=docs/building.md no longer publishes the install command \
                     this build reports: {command}"
                );
            }
        }
    }

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
    pub fn build_vendored(out_dir: &Path, toolchain: &Toolchain) -> Libs {
        let vendor = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("vendor/arjun/upstream");
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

    /// The preprocessor definitions the vendored CaDiCaL library is built with.
    ///
    /// `Internal` and `Stats` have conditionally compiled members, so a
    /// translation unit that includes `internal.hpp` reads the wrong offsets
    /// unless it is compiled with the same set. This list is checked against
    /// CaDiCaL's own `CMakeLists.txt` below rather than trusted, so a define
    /// added or removed upstream fails the build instead of silently changing
    /// what the stats accessor returns.
    ///
    /// `NDEBUG` is not in the checked set: CMake supplies it through the
    /// `Release` build type, which `build_vendored` selects, rather than
    /// through `target_compile_definitions`.
    const CADICAL_DEFINES: [&str; 5] = [
        "NCONTRACTS",
        "NTRACING",
        "NBUILD",
        "NCLOSEFROM",
        "NUNLOCKED",
    ];

    /// Compile the one translation unit that reaches into CaDiCaL's internals.
    ///
    /// Separate from the shims above because it needs CaDiCaL's own define set
    /// and its own language standard: the library is built at C++17, and this
    /// file is compiled from the same headers, so it is compiled the same way.
    fn compile_internal_stats(out_dir: &Path, toolchain: &Toolchain, libs: &Libs) -> PathBuf {
        let cmake_lists = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("vendor/arjun/upstream/cadical/CMakeLists.txt");
        let cmake = std::fs::read_to_string(&cmake_lists).expect("read CaDiCaL CMakeLists.txt");
        for def in CADICAL_DEFINES {
            assert!(
                cmake.contains(def),
                "{} no longer defines {def}, which `cadical_internal_stats.cpp` is \
                 compiled with — the two must agree or the stats accessor reads the \
                 wrong struct offsets",
                cmake_lists.display(),
            );
        }

        let src = "cadical_internal_stats.cpp";
        let obj = out_dir.join(src.replace(".cpp", ".o"));
        let mut tu = Command::new(&toolchain.cxx);
        tu.arg("-std=c++17")
            .arg(format!("-O{CXX_OPT_LEVEL}"))
            .args(["-fPIC", "-c"])
            .args(toolchain.cxx_flags);
        for def in CADICAL_DEFINES {
            tu.arg(format!("-D{def}"));
        }
        tu.arg("-DNDEBUG");
        for inc in &libs.includes {
            tu.arg("-I").arg(inc);
        }
        tu.arg("-Ivendor/arjun")
            .arg(format!("vendor/arjun/{src}"))
            .arg("-o")
            .arg(&obj);
        run(tu, &format!("compile {src}"));
        obj
    }

    /// Compile the C shims and fold the whole stack into ONE static archive.
    ///
    /// Static, not a shared object, because a `.so` here can only live in
    /// `OUT_DIR` — and nothing keeps `OUT_DIR` alive. `cargo install` discards
    /// its build directory outright, so the installed binary starts and
    /// immediately dies with a loader error; anyone who builds a tool against
    /// this crate and then ships the executable hits the same thing, since
    /// `OUT_DIR` does not travel with it.
    ///
    /// This is only safe because exactly one CaDiCaL is in the process. Linking
    /// two same-version-but-different CaDiCaLs statically would merge COMDAT
    /// groups (vtables, libstdc++ template instantiations) that cannot be
    /// separated after the fact — which is why an earlier revision isolated
    /// Arjun's copy behind a version-scripted `.so`. Our preprocessing now uses
    /// the vendored fork through `cadical_shim`, so there is nothing to isolate.
    ///
    /// The six archives reference each other cyclically (arjun <-> cms <->
    /// cadical). A single merged archive handles that without `--start-group`:
    /// the linker re-scans one archive until it reaches closure. Merging is also
    /// what makes this work for *dependents* — `rustc-link-lib=static=` is
    /// recorded in crate metadata and propagates, whereas `rustc-link-arg`
    /// (which passing loose archive paths would need) does not.
    pub fn link_shim(out_dir: &Path, toolchain: &Toolchain, libs: &Libs) {
        let mut objects: Vec<PathBuf> = Vec::new();
        for src in toolchain.shims {
            let obj = out_dir.join(src.replace(".cpp", ".o"));
            let mut shim = Command::new(&toolchain.cxx);
            shim.arg("-std=c++20")
                .arg(format!("-O{CXX_OPT_LEVEL}"))
                .args(["-fPIC", "-c"])
                .args(toolchain.cxx_flags);
            for inc in &libs.includes {
                shim.arg("-I").arg(inc);
            }
            shim.arg("-Ivendor/arjun")
                .arg(format!("vendor/arjun/{src}"))
                .arg("-o")
                .arg(&obj);
            run(shim, &format!("compile {src}"));
            objects.push(obj);
        }
        objects.push(compile_internal_stats(out_dir, toolchain, libs));

        // `ar -M` (MRI script) is the portable way to concatenate archives:
        // `addlib` splices in every member of an existing .a, `addmod` adds a
        // loose object.
        let merged = out_dir.join("libvitri_arjun.a");
        let _ = std::fs::remove_file(&merged);
        let mut mri = format!("create {}\n", merged.display());
        for a in &libs.archives {
            mri.push_str(&format!("addlib {}\n", a.display()));
        }
        for o in &objects {
            mri.push_str(&format!("addmod {}\n", o.display()));
        }
        mri.push_str("save\nend\n");

        let script = out_dir.join("merge.mri");
        std::fs::write(&script, &mri).expect("write ar MRI script");
        let mut merge = Command::new(&toolchain.ar);
        merge.arg("-M").stdin(std::process::Stdio::piped());
        run_with_stdin(merge, &mri, "merge static archives");

        let out = out_dir.display();
        println!("cargo:rustc-link-search=native={out}");
        println!("cargo:rustc-link-lib=static=vitri_arjun");
        for dir in &toolchain.dylib_dirs {
            println!("cargo:rustc-link-search=native={}", dir.display());
        }
        for lib in toolchain.dylibs {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
        if !toolchain.side_modules.is_empty() {
            let paths = std::env::join_paths(&toolchain.side_modules)
                .expect("a side module path contains the path-list separator");
            println!("cargo::metadata=side_modules={}", paths.to_string_lossy());
        }
        // libgcc_s stays dynamic deliberately: Rust's panic=unwind OOM recovery
        // relies on it, so we do NOT force -static-libgcc.
    }

    /// The upstream Arjun commit these sources were vendored at, read from the
    /// `ARJUN_PIN_SHA1` text file beside them and passed to CMake as
    /// `-DGIT_SHA1=`. A vendored tree has no `.git`, so upstream's own probe
    /// would leave the built library reporting an empty version; this makes it
    /// report the commit recorded in `PROVENANCE.md`. The file lives inside the
    /// package because `include` cannot reach outside the crate root.
    pub fn arjun_pin(vendor: &Path) -> String {
        let p = vendor.join("ARJUN_PIN_SHA1");
        std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
            .trim()
            .to_string()
    }

    fn run(mut cmd: Command, what: &str) {
        let status = cmd
            .status()
            .unwrap_or_else(|e| panic!("failed to spawn {what}: {e} (command: {cmd:?})"));
        assert!(status.success(), "{what} failed (command: {cmd:?})");
    }

    /// Same, for a command driven by a script on stdin (`ar -M`).
    fn run_with_stdin(mut cmd: Command, input: &str, what: &str) {
        use std::io::Write;

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {what}: {e} (command: {cmd:?})"));
        child
            .stdin
            .as_mut()
            .unwrap_or_else(|| panic!("{what}: stdin was not piped"))
            .write_all(input.as_bytes())
            .unwrap_or_else(|e| panic!("failed to write {what} script: {e}"));
        let status = child
            .wait()
            .unwrap_or_else(|e| panic!("failed to wait for {what}: {e}"));
        assert!(status.success(), "{what} failed (command: {cmd:?})");
    }
}
