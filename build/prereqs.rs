//! What the vendored C++ build needs installed, checked before either half of
//! it starts.
//!
//! One table: the check reads it, every failure message prints install commands
//! built from it, and `docs/building.md` publishes those same commands.

use std::path::Path;
use std::process::Command;

use crate::run::have;

/// How the build looks for one prerequisite.
enum Probe {
    /// The named program answers `--version`.
    OnPath(&'static str),
    /// The chosen C and C++ compilers both answer `--version`. Which
    /// programs those are comes from [`crate::toolchain::find_cxx`], not from the table.
    Compiler,
    /// A one-file program using all three libraries compiles and links.
    Links,
}

/// Which builds need a prerequisite: the native one, the one for Emscripten,
/// or both.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needed {
    Both,
    Native,
    Emscripten,
}

impl Needed {
    /// Whether the build about to run has to have this one.
    fn covers(self, emscripten: bool) -> bool {
        match self {
            Needed::Both => true,
            Needed::Native => !emscripten,
            Needed::Emscripten => emscripten,
        }
    }
}

/// One prerequisite of the vendored C++ build: which builds need it, how the
/// build looks for it, and the package that carries it in each package manager
/// the failure message offers.
struct Prereq {
    /// What the build looks for, worded as the message names it.
    what: &'static str,
    needed: Needed,
    probe: Probe,
    apt: &'static str,
    dnf: &'static str,
    /// Where it comes from when no package manager carries it; empty when the
    /// install commands the message prints already cover it.
    otherwise: &'static str,
}

/// THE prerequisite list: [`require_prereqs`] checks these in order, every
/// failure message prints install commands built from them, and
/// `docs/building.md` publishes those same commands.
const PREREQS: &[Prereq] = &[
    Prereq {
        what: "a C++20 compiler (gcc 12 or newer)",
        needed: Needed::Native,
        probe: Probe::Compiler,
        apt: "build-essential gcc-12 g++-12",
        dnf: "gcc-c++",
        otherwise: "",
    },
    Prereq {
        what: "the Emscripten C++ compiler",
        needed: Needed::Emscripten,
        probe: Probe::OnPath("em++"),
        apt: "",
        dnf: "",
        otherwise: SDK,
    },
    Prereq {
        what: "the Emscripten archiver",
        needed: Needed::Emscripten,
        probe: Probe::OnPath("emar"),
        apt: "",
        dnf: "",
        otherwise: SDK,
    },
    Prereq {
        what: "CMake",
        needed: Needed::Both,
        probe: Probe::OnPath("cmake"),
        apt: "cmake",
        dnf: "cmake",
        otherwise: "",
    },
    Prereq {
        what: "pkg-config",
        needed: Needed::Both,
        probe: Probe::OnPath("pkg-config"),
        apt: "pkg-config",
        dnf: "pkgconf-pkg-config",
        otherwise: "",
    },
    Prereq {
        what: "the GMP, MPFR and zlib development packages",
        needed: Needed::Native,
        probe: Probe::Links,
        apt: "libgmp-dev libmpfr-dev zlib1g-dev",
        dnf: "gmp-devel mpfr-devel zlib-devel",
        otherwise: "",
    },
];

/// What to do about a tool the Emscripten SDK carries and no package manager
/// does.
const SDK: &str = "Install the Emscripten SDK and source its emsdk_env.sh \
                   (docs/building.md).";

/// Check every prerequisite, before either half of the build starts.
///
/// Unconditional, because none of the ways [`crate::toolchain::find_cxx`] can arrive at a
/// compiler implies that CMake, pkg-config or the system libraries are
/// installed. Three `--version` runs and one small compile buy the
/// difference between a sentence naming the missing package and a CMake
/// configure error, or a wall of linker noise minutes into the build.
pub(crate) fn require_prereqs(out_dir: &Path, compilers: Option<(&str, &str)>) {
    let emscripten = compilers.is_none();
    for prereq in PREREQS.iter().filter(|p| p.needed.covers(emscripten)) {
        let wrong = match prereq.probe {
            // Both halves: CMake configures the vendored projects with a C
            // compiler as well, and [`crate::toolchain::find_cxx`] derives its name from the
            // C++ one rather than being told it.
            Probe::Compiler => {
                let (cc, cxx) = compilers.expect("a native build names its compilers");
                (!have(cxx) || !have(cc)).then(|| {
                    format!(
                        "the build needs `{cxx}` and `{cc}`, and at least one does not \
                         run. Install them, or name the C++ one in VITRI_CXX"
                    )
                })
            }
            Probe::OnPath(tool) => (!have(tool)).then(|| format!("`{tool}` is not on PATH")),
            Probe::Links => {
                let (_, cxx) = compilers.expect("a native build names its compilers");
                (!links_system_libs(out_dir, cxx))
                    .then(|| format!("at least one is missing or unusable with `{cxx}`"))
            }
        };
        if let Some(detail) = wrong {
            let how = if prereq.otherwise.is_empty() {
                format!(
                    "Install every prerequisite with one of:\n{}",
                    install_commands()
                )
            } else {
                prereq.otherwise.to_string()
            };
            panic!(
                "vitri's vendored C++ stack needs {}, and {detail}.\n{how}\n\
                 docs/building.md says what each one is for.",
                prereq.what,
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
