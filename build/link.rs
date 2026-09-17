//! This crate's own translation units, and the single static archive the whole
//! stack is folded into.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CXX_OPT_LEVEL;
use crate::run::{run, run_with_stdin};
use crate::toolchain::{Libs, Toolchain};

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

    let mut defines: Vec<&str> = CADICAL_DEFINES.to_vec();
    defines.push("NDEBUG");
    compile_tu(
        out_dir,
        toolchain,
        libs,
        "cadical_internal_stats.cpp",
        "c++17",
        &defines,
    )
}

/// Compile one of this crate's translation units under `vendor/arjun/` into
/// `OUT_DIR`, and return the object.
///
/// The language standard and the defines are what the two callers disagree on.
/// Everything else is the same compile: the same compiler, optimisation level,
/// target flags and include path, so a change to any of those reaches both.
fn compile_tu(
    out_dir: &Path,
    toolchain: &Toolchain,
    libs: &Libs,
    src: &str,
    std: &str,
    defines: &[&str],
) -> PathBuf {
    let obj = out_dir.join(src.replace(".cpp", ".o"));
    let mut tu = Command::new(&toolchain.cxx);
    tu.arg(format!("-std={std}"))
        .arg(format!("-O{CXX_OPT_LEVEL}"))
        .args(["-fPIC", "-c"])
        .args(toolchain.cxx_flags);
    for def in defines {
        tu.arg(format!("-D{def}"));
    }
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
pub(crate) fn link_shim(out_dir: &Path, toolchain: &Toolchain, libs: &Libs) {
    let mut objects: Vec<PathBuf> = Vec::new();
    for src in toolchain.shims {
        objects.push(compile_tu(out_dir, toolchain, libs, src, "c++20", &[]));
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
