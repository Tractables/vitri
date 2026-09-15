//! Puts GMP's side modules on the module's link line.
//!
//! vitri links GMP dynamically, as side modules, but emcc links a side module
//! only from its path, and link arguments from a dependency's build script do
//! not reach this program's link. vitri's build script publishes the paths.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let paths = std::env::var_os("DEP_VITRI_ARJUN_SIDE_MODULES")
        .expect("vitri's build script names GMP's side modules in a build for Emscripten");
    for path in std::env::split_paths(&paths) {
        println!("cargo::rustc-link-arg-bins={}", path.display());
    }
}
