//! Build script: compiles the two vendored C++ components. Both are always
//! built — there is no configuration in which this crate ships without them.
//!
//! * The BSD-2 FlowCutter tree-decomposition builder, a handful of translation
//!   units driven straight by `cc`.
//! * The Arjun preprocessing stack, five CMake projects vendored under
//!   `vendor/arjun/upstream/`. `build.rs` drives CMake itself, so a plain
//!   `cargo build` produces a working Arjun with no install script, no network
//!   access and no out-of-tree state. See `vendor/arjun/upstream/PROVENANCE.md`.
//!
//! Everything a published crate needs is inside the package: `cargo build` from
//! a freshly unpacked `.crate` works with the network unavailable, because the
//! vendored sources are complete and CMake runs with
//! `FETCHCONTENT_FULLY_DISCONNECTED=ON`.

use std::path::{Path, PathBuf};

/// The optimisation level every vendored C++ translation unit is compiled at —
/// both halves of the build and the shim objects that join them. Release code
/// only: nothing here is stepped through in a debugger.
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

    // One compiler choice, one prerequisite check, both halves of the build.
    println!("cargo:rerun-if-env-changed=VITRI_CXX");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let (cc, cxx) = arjun::find_cxx();
    arjun::require_prereqs(&out_dir, &cxx);

    build_treedecomp(&cxx);
    build_arjun(&out_dir, &cc, &cxx);
}

// ---------------------------------------------------------------- treedecomp

fn build_treedecomp(cxx: &str) {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        // The compiler comes from `find_cxx`, not from `cc`'s own `CXX`
        // lookup: one build cannot be half one compiler's C++ and half
        // another's, and `VITRI_CXX` is documented as choosing the compiler
        // for the build rather than for one part of it.
        .compiler(cxx)
        .std("c++20")
        .opt_level(CXX_OPT_LEVEL)
        .define("NDEBUG", None)
        .warnings(false)
        .include("vendor/treedecomp")
        .include("vendor/treedecomp/upstream")
        .include("vendor/treedecomp/upstream/flow-cutter-pace17/src")
        .file("vendor/treedecomp/ffi.cpp")
        .file("vendor/treedecomp/heap_selftest.cpp")
        .file("vendor/treedecomp/upstream/IFlowCutter.cpp")
        .file("vendor/treedecomp/upstream/TreeDecomposition.cpp")
        .file("vendor/treedecomp/upstream/graph.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/cell.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/greedy_order.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/tree_decomposition.cpp");
    // Note: pace.cpp excluded — it contains main(). IFlowCutter.cpp has all needed logic.

    build.compile("treedecomp");

    println!("cargo:rerun-if-changed=vendor/treedecomp/");
}

// ------------------------------------------------- vendored C++ SAT stack
//
// One CMake build produces everything downstream of it: Arjun (plus
// CryptoMiniSat and CadiBack) requires meelgroup's CaDiCaL fork, and our own
// preprocessing links that same fork through `cadical_shim` instead of a
// second, stock copy — so there is exactly one CaDiCaL in the process.

fn build_arjun(out_dir: &Path, cc: &str, cxx: &str) {
    println!("cargo:rerun-if-changed=vendor/arjun/");

    let libs = arjun::build_vendored(out_dir, cc, cxx);

    arjun::link_shim(out_dir, cxx, &libs);
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
    pub fn build_vendored(out_dir: &Path, cc: &str, cxx: &str) -> Libs {
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

        let mut cfg = Command::new("cmake");
        cfg.arg("-S")
            .arg(vendor.join("arjun"))
            .arg("-B")
            .arg(&build_dir)
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(format!("-DCMAKE_C_COMPILER={cc}"))
            .arg(format!("-DCMAKE_CXX_COMPILER={cxx}"))
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
            "-DCMAKE_CXX_FLAGS=-DEIGEN_MPL2_ONLY -O{CXX_OPT_LEVEL}"
        ))
        // The vendored tree has no .git, so Arjun's own git probe would bake
        // an EMPTY "Arjun SHA1:" into the binary — the identity every
        // consumer checks to spot a stale or foreign install. Pass the pin
        // explicitly; `arjun/CMakeLists.txt` was modified to honour it.
        .arg(format!("-DGIT_SHA1={}", arjun_pin(&vendor)));
        run(cfg, "cmake configure (Arjun stack)");

        let mut build = Command::new("cmake");
        build.arg("--build").arg(&build_dir);
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
            ],
            archives,
        }
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
    pub fn link_shim(out_dir: &Path, cxx: &str, libs: &Libs) {
        // arjun_shim exposes Arjun itself; cadical_shim backs our own
        // preprocessing. Both are part of every build.
        let sources: [&str; 2] = ["cadical_shim.cpp", "arjun_shim.cpp"];

        let mut objects: Vec<PathBuf> = Vec::new();
        for src in &sources {
            let obj = out_dir.join(src.replace(".cpp", ".o"));
            let mut shim = Command::new(cxx);
            shim.arg("-std=c++20")
                .arg(format!("-O{CXX_OPT_LEVEL}"))
                .args(["-fPIC", "-c"]);
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
        println!("cargo:rerun-if-env-changed=AR");
        let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
        let mut merge = Command::new(&ar);
        merge.arg("-M").stdin(std::process::Stdio::piped());
        run_with_stdin(merge, &mri, "merge static archives");

        let out = out_dir.display();
        println!("cargo:rustc-link-search=native={out}");
        println!("cargo:rustc-link-lib=static=vitri_arjun");
        // The C++ runtime and the numeric libraries stay dynamic, for the
        // reason `links_system_libs` gives. There is deliberately no switch.
        println!("cargo:rustc-link-lib=dylib=stdc++");
        for lib in ["gmpxx", "gmp", "mpfr", "z"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
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
