//! Rebuild SDL when its tree or our configuration changes.
//!
//! sdl3-sys's own build script never reruns for a registry crate, and Cargo skips gitignored files
//! when fingerprinting a path crate, so edits in third_party/SDL would otherwise go unbuilt. A rerun
//! here changes this crate, which reruns sdl3-sys's build (CMake then rebuilds only what changed).
//! Not the whole tree: its .git changes on every `git status`.

use std::path::Path;

fn main() {
    let sdl = Path::new(env!("CARGO_MANIFEST_DIR")).join("../SDL");
    if !sdl.join("CMakeLists.txt").is_file() {
        panic!(
            "third_party/SDL is missing: run scripts/sdl.sh fetch (once; it clones the pinned SDL \
             release and applies patches/sdl-patchbox.patch). Or build with the system SDL: \
             cargo build --no-default-features"
        );
    }
    for p in ["CMakeLists.txt", "cmake", "include", "src", "wayland-protocols"] {
        println!("cargo::rerun-if-changed=../SDL/{p}");
    }
    println!("cargo::rerun-if-changed=cmake");
}
