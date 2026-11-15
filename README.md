# PatchBox

A patchbay, audio server control and session manager in one program, for JACK and PipeWire on
Linux. It shows and edits JACK's graph and PipeWire's, drives the JACK server through jackdbus and
PipeWire's clock and devices through PipeWire's own tools, and runs sessions of NSM clients and plain
programs. Rust, SDL3 (vendored, static, Wayland only) and a GL renderer of its own, with no
toolkit.

PatchBox is built alongside [JackBus](https://github.com/nine7nine/JackBus), a JACK2
fork for real-time work, and uses what it adds: an application's clients leave the graph before
the application is stopped, and the buffer size and CPU wake-up latency change without restarting
the server. It doesn't need it:

| The set-up | What PatchBox does there |
|---|---|
| JackBus's jackdbus, with or without PipeWire on top | everything: JACK's graph, server control, sessions; with PipeWire bridged, its graph (opened from its JACK client) and its settings tab too |
| PipeWire alone (pipewire-jack) | the patchbay (PipeWire's nodes through its JACK API), sessions, PipeWire's settings; no jackdbus page |
| another jackdbus | the same, except that a client is stopped without leaving the graph first, and a setting that can't change live restarts the server |
| a plain jackd | the patchbay, sessions and transport; no server control |

**[Architecture document](https://nine7nine.github.io/PatchBox/)**: how the window, the session
daemon, JACK, jackdbus and PipeWire fit together, with diagrams and measurements (source:
[`docs/index.html`](docs/index.html)). Every control is described in the
[manual](docs/MANUAL.md).

## Why

Patching, the server's settings and the session usually take several programs: a server panel
(qjackctl), a patchbay (Patchage, Catia, PatchMatrix, qpwgraph, Helvum…), a session manager (NSM,
RaySession, Agordejo…). PatchBox puts them in one window, with a daemon that keeps the session
running when the window closes. It borrows ideas from many of these programs (connection
matrices, session behaviour, connection rules) and implements them anew, with a session format of
its own.

## Features

- **Patchbay.** Two views of the graph: Ports (each block shows its ports, a wire per connection)
  and Matrix (a connection matrix per pair of clients). Audio, MIDI, CV and OSC apart. Stereo
  pairs detected; port groups that connect 1:1; Ardour's tracks and Carla's plugins split into
  blocks of their own; your names for clients and ports, exported to other programs as JACK
  metadata; find; selection, alignment, hiding; a layout and camera per view.
- **PipeWire.** With PipeWire on top of JACK, its own graph (desktop streams, devices, audio,
  MIDI, video) opens from its JACK client and is edited the same way, never mixed into JACK's.
  With PipeWire as the JACK server, its nodes are the patchbay's. A settings tab for either: the
  clock, default devices, device profiles, WirePlumber's settings, the log level, a restart.
- **Connection rules.** Connections kept whenever both ports exist, with or without a session
  (`*` wildcards); one you remove by hand stays removed.
- **Sessions.** `patchboxd`, an NSM server: NSM clients (Ardour, Carla, Hydrogen…) told to open
  and save, plain programs started and stopped with the session, its connections restored as
  ports appear. Templates, snapshots, client templates, per-client hooks, notes. Deleted things
  go to the desktop's Trash. `patchbox-ctl` drives it all from a shell.
- **JACK server.** Start, stop, restart; every driver and engine parameter jackdbus offers, with
  its constraints; presets switched in one click; a session can carry the server's settings.
  Xruns, DSP load, the event log and jackdbus's own log; transport.

## Requirements

- Linux with a Wayland compositor.
- libjack: JACK2's, or PipeWire's (pipewire-jack). jackdbus for JACK server control, ideally
  [JackBus](https://github.com/nine7nine/JackBus).
- Optional: PipeWire and WirePlumber (PipeWire's graph and tab), git (session snapshots), xdg-utils.
- To build: Rust (edition 2024), CMake, pkgconf, wayland, libxkbcommon, EGL (mesa), dbus; librsvg
  for the package's icons.

## Build

```sh
scripts/sdl.sh fetch     # once: the pinned SDL release + patches/sdl-patchbox.patch (network)
cargo build --release    # builds our static SDL too (no libSDL3.so at run time)
cargo test
```

`--no-default-features` links the system's SDL instead. See the manual's
[static SDL](docs/MANUAL.md#the-static-sdl) section.

## Install

Arch Linux: `dist/arch/PKGBUILD` builds the package from this clone:

```sh
cd dist/arch
makepkg -si
```

It installs `patchbox` (the window), `patchboxd` (the session daemon), `patchbox-ctl`, the desktop
entry and icons, and a systemd user unit for running sessions without the window
(`systemctl --user enable --now patchboxd`; not needed otherwise: the window starts the daemon
when none runs).

Elsewhere: build as above and install `target/release/{patchbox,patchboxd,patchbox-ctl}` and the
files in `dist/share/`.

## Getting started

Start PatchBox with the audio server running: jackdbus, jackd, or PipeWire with pipewire-jack. It
doesn't start one by itself, and reconnects when the server comes back.

- **1–4** choose the port type (AUDIO, MIDI, CV, OSC), **M** switches between the Ports and
  Matrix views, **Ctrl+F** finds a block or port, **F** fits the graph, **F9** shows the sidebar.
- Drag a port onto another to connect or disconnect them; in a matrix, click or paint cells.
- The sidebar's session section: **New**, then **+ Application** to add programs.
- The JACK section: start and stop the server, **Settings** for its parameters and presets.

Every control, the session format, the settings and the test harness are described in
[docs/MANUAL.md](docs/MANUAL.md); how it is built, in the
[architecture document](https://nine7nine.github.io/PatchBox/).

## Testing

The tests never touch the live JACK server or the desktop: they run against a private jackd or
jackdbus inside their own user, IPC, mount and PID namespaces, the GUI in a nested
`kwin_wayland --virtual`.

```sh
cargo test
tests/session/run.sh       # patchboxd end to end: sessions, an NSM client, connection memory
tests/nested/run.sh        # the GUI, scripted, against fake clients
```

Sanitizer builds and the other harness options: see the manual.

## Licence

GPL-2.0-or-later (`LICENSE`). Ideas from other patchbays and session managers, among them
RaySession (GPL-2.0) and PatchMatrix (Artistic-2.0), are reimplemented, not copied. Cousine font: Apache-2.0 (`assets/fonts/LICENSE`). SDL: zlib.
