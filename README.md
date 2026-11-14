# PatchBox

A JACK patchbay, JACK server control and an NSM session manager in one program, for audio work on
Linux. The graph keeps PatchMatrix's look and its connection matrices; RaySession's flexibility is
underneath it and its session management beside it; the JACK server is driven through jackdbus.
Rust, SDL3 (vendored, static, Wayland only) and a GL renderer of its own, with no toolkit.

PatchBox is built alongside [jack2dbus-nspa](https://github.com/nine7nine/jack2dbus-nspa), a JACK2
fork for real-time work, and uses what it adds: an application's clients leave the graph before
the application is stopped, and the buffer size and CPU wake-up latency change without restarting
the server. With another jackdbus it works with what that offers; with a plain jackd the patchbay
and transport work, server control doesn't.

## Why

It replaces three programs: qjackctl for the server, RaySession for sessions, and PatchMatrix or
Catia for the graph. One window, one place for the server's settings, the graph and the session,
and a daemon that keeps the session running when the window closes.

## Features

- **Patchbay.** Two views of the JACK graph: Ports (each block shows its ports, a wire per
  connection) and Matrix (PatchMatrix's: one connection matrix per pair of clients). Audio, MIDI,
  CV and OSC apart. Stereo pairs detected; port groups that connect 1:1; Ardour's tracks and
  Carla's plugins split into blocks of their own; your names for clients and ports, exported to
  other programs as JACK metadata; find; selection, alignment, hiding; a layout per view.
- **Connection rules.** Connections kept whenever both ports exist, with or without a session
  (`*` wildcards); one you remove by hand stays removed.
- **Sessions.** `patchboxd`, an NSM server: NSM clients (Ardour, Carla, Hydrogen…) told to open
  and save, plain programs started and stopped with the session, its connections restored as
  ports appear. Templates, snapshots, client templates, per-client hooks, notes. Deleted things
  go to the desktop's Trash. `patchbox-ctl` drives it all from a shell.
- **JACK server.** Start, stop, restart; every driver and engine parameter jackdbus offers, with
  its constraints; presets switched in one click; a session can carry the server's settings.
  Xruns, DSP load, the event log and jackdbus's own log; transport.
- **PipeWire.** A settings tab for PipeWire on top of JACK (the bridge) or on its own: the clock,
  default devices, device profiles, WirePlumber's settings, the log level, a restart.

## Requirements

- Linux with a Wayland compositor.
- JACK2's libjack; jackdbus for server control, ideally
  [jack2dbus-nspa](https://github.com/nine7nine/jack2dbus-nspa).
- Optional: PipeWire and WirePlumber (the PipeWire tab), git (session snapshots), xdg-utils.
- To build: Rust (edition 2024), CMake, pkgconf, wayland, libxkbcommon, EGL (mesa), dbus; librsvg
  for the package's icons.

## Build

```sh
scripts/sdl.sh fetch     # once: the pinned SDL release + patches/sdl-patchbox.patch (network)
cargo build --release    # builds our static SDL too (no libSDL3.so at run time)
cargo test
```

`--no-default-features` links the system's SDL instead. See `docs/sdl-vendoring.md`.

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

Start PatchBox with the JACK server running (it never starts one, and reconnects when the server
comes back).

- **1–4** choose the port type (AUDIO, MIDI, CV, OSC), **M** switches between the Ports and
  Matrix views, **Ctrl+F** finds a block or port, **F** fits the graph, **F9** shows the sidebar.
- Drag a port onto another to connect or disconnect them; in a matrix, click or paint cells.
- The sidebar's session section: **New**, then **+ Application** to add programs.
- The JACK section: start and stop the server, **Settings** for its parameters and presets.

Every control, the session format, the settings and the test harness are described in
[docs/MANUAL.md](docs/MANUAL.md). `docs/PLAN.md` is the design and what comes next.

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

GPL-2.0-or-later (`LICENSE`). Ideas taken from RaySession (GPL-2.0) and PatchMatrix (Artistic-2.0)
are reimplemented, not copied. Cousine font: Apache-2.0 (`assets/fonts/LICENSE`). SDL: zlib.
