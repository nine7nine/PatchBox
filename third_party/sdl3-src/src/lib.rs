//! The SDL source sdl3-sys builds (see Cargo.toml): our CMake wrapper around third_party/SDL.

/// What sdl3-sys hands to CMake: the wrapper with our configuration (cmake/CMakeLists.txt).
pub const SOURCE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/cmake");

/// SDL_GetRevision() of this build: the pinned upstream tag and commit (scripts/sdl.sh), and that
/// it is ours. A git checkout has no REVISION.txt, so SDL takes this from sdl3-sys's -DSDL_REVISION.
pub const REVISION: &str = "SDL-release-3.4.18-829a65d7";
