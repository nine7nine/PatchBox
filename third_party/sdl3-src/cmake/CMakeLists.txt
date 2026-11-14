# patchbox's SDL configuration (docs/sdl-vendoring.md).
#
# sdl3-sys configures this directory (sdl3_src::SOURCE_DIR) instead of SDL's own, and links what it
# installs. It builds third_party/SDL as a subproject with the options below, which override both
# SDL's defaults and the -D flags sdl3-sys passes. Re-check what we use with `grep -rn sdl3 src/`
# before changing anything here.
cmake_minimum_required(VERSION 3.16...4.4)
project(patchbox_sdl LANGUAGES C)

macro(patchbox_sdl name value)
  set(${name} ${value} CACHE BOOL "patchbox (third_party/sdl3-src/cmake)" FORCE)
endmacro()

# One static library, installed into sdl3-sys's OUT_DIR (sdl3.pc and SDL3Config.cmake are how it
# finds the library). As a subproject SDL defaults to installing nothing.
patchbox_sdl(SDL_SHARED OFF)
patchbox_sdl(SDL_STATIC ON)
patchbox_sdl(SDL_INSTALL ON)
patchbox_sdl(SDL_INSTALL_CPACK OFF)
patchbox_sdl(SDL_INSTALL_DOCS OFF)
patchbox_sdl(SDL_UNINSTALL OFF)
patchbox_sdl(SDL_TESTS OFF)
patchbox_sdl(SDL_TEST_LIBRARY OFF)
patchbox_sdl(SDL_EXAMPLES OFF)
set(SDL_VENDOR_INFO "patchbox" CACHE STRING "" FORCE)

# Link what we use directly instead of dlopen()ing it at run time (the system libSDL3.so can dlopen
# ~30 libraries: X11 and its extensions, five audio stacks, Vulkan, udev, libusb...). EGL and the
# GL library are still loaded by SDL_GL_LoadLibrary, as everywhere.
patchbox_sdl(SDL_DEPS_SHARED OFF)

# What PatchBox uses: video on Wayland, OpenGL through EGL (gfx.rs draws everything), events,
# keyboard/text input, mouse, clipboard, hints. No audio: JACK is ours (libjack), not SDL's.
patchbox_sdl(SDL_VIDEO ON)
patchbox_sdl(SDL_AUDIO OFF)
patchbox_sdl(SDL_CAMERA OFF)
patchbox_sdl(SDL_RENDER OFF)
patchbox_sdl(SDL_GPU OFF)
patchbox_sdl(SDL_JOYSTICK OFF)
patchbox_sdl(SDL_HAPTIC OFF)
patchbox_sdl(SDL_HIDAPI OFF)
patchbox_sdl(SDL_SENSOR OFF)
patchbox_sdl(SDL_POWER OFF)
patchbox_sdl(SDL_DIALOG OFF)
patchbox_sdl(SDL_TRAY OFF)

# Wayland only: no X11. No decorations from SDL (csd.rs draws ours), no KMS/DRM,
# Vulkan or GLES.
patchbox_sdl(SDL_WAYLAND ON)
patchbox_sdl(SDL_WAYLAND_LIBDECOR OFF)
patchbox_sdl(SDL_X11 OFF)
patchbox_sdl(SDL_KMSDRM OFF)
patchbox_sdl(SDL_OFFSCREEN OFF)
patchbox_sdl(SDL_DUMMYVIDEO OFF)
patchbox_sdl(SDL_VULKAN OFF)
patchbox_sdl(SDL_RENDER_VULKAN OFF)
patchbox_sdl(SDL_RENDER_GPU OFF)
patchbox_sdl(SDL_OPENGL ON)
patchbox_sdl(SDL_OPENGLES OFF)

# Platform glue. No D-Bus: with the cursor-shape protocol (KWin, GNOME 46+) the compositor draws
# the cursors, so SDL's portal lookup of the cursor theme isn't used; input methods go through
# Wayland's text-input-v3; we let the screensaver be (SDL_VIDEO_ALLOW_SCREENSAVER) and use
# neither the colour-scheme portal nor SDL_OpenURL. So libdbus isn't loaded at all. No IBus, udev
# (joystick hotplug), io_uring (the async I/O API), libthai/fribidi (X11 message boxes), audio
# backends.
patchbox_sdl(SDL_DBUS OFF)
patchbox_sdl(SDL_IBUS OFF)
patchbox_sdl(SDL_LIBUDEV OFF)
patchbox_sdl(SDL_LIBURING OFF)
patchbox_sdl(SDL_LIBTHAI OFF)
patchbox_sdl(SDL_FRIBIDI OFF)
patchbox_sdl(SDL_PIPEWIRE OFF)
patchbox_sdl(SDL_PULSEAUDIO OFF)
patchbox_sdl(SDL_ALSA OFF)
patchbox_sdl(SDL_JACK OFF)
patchbox_sdl(SDL_SNDIO OFF)
patchbox_sdl(SDL_OSS OFF)

# A section per function and per datum: the linker drops what PatchBox never calls (blitters,
# image loading, YUV, the render API's core…) instead of whole files.
string(APPEND CMAKE_C_FLAGS " -ffunction-sections -fdata-sections")

add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../SDL" sdl)
