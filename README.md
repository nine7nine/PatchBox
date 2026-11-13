# PatchBox

A JACK patchbay — and, next, session manager — for one Linux audio box: the graph of PatchMatrix
(client nodes with connection matrices between them, the owner's high-contrast colours), with
RaySession's flexibility underneath and its session management beside it. Rust, SDL3 (vendored,
static, Wayland-only) and GL drawn by the app itself. No Qt.

Status: phase 1 of `docs/PLAN.md` — the live patchbay works; sessions come in phase 3.

## Build

```sh
scripts/sdl.sh fetch     # once: the pinned SDL release + patches/sdl-patchbox.patch (network)
cargo build --release    # builds our static SDL too (no libSDL3.so at run time)
cargo test
```

Needs libjack (jack2), wayland, EGL, xkbcommon, CMake. `--no-default-features` links the system's
SDL instead (A/B runs). See `docs/sdl-vendoring.md`.

## Use

PatchBox connects to the running JACK server (it never starts one) and reconnects when the server
comes back. Its JACK client has no process callback: jack2 starts no RT thread for it and never
puts it in the graph.

| | |
|---|---|
| Top bar | port type (AUDIO / MIDI / CV / OSC, or keys 1–4), sample rate and latency, buffer size (menu), xruns (click: reset), DSP load |
| Matrix cell | click: connect / disconnect; drag across cells: paint the same state; **Shift**: the stereo partner cell too |
| Node | drag: move (its matrices follow halfway); right-click: split/join inputs and outputs, disconnect, hide |
| Output handle → node | drag from a node's right handle (or Shift+drag the node) onto another node: open a matrix between them; hold **Ctrl** at the drop to connect 1:1 |
| Matrix | Ctrl+drag: move; right-click: connect 1:1, disconnect all, remove |
| Canvas | drag or middle-drag: pan; wheel: scroll; Ctrl+wheel: zoom; F: fit; Ctrl+0: zoom 1:1; L: port names on every matrix (hovering shows a matrix's names anyway); F9: sidebar |
| Sidebar | clients (click: find; right-click: menu, show hidden ones), log |

Physical ports and a2j devices start split into a capture node and a playback node (PatchMatrix's
and RaySession's default); any client can be split or joined. Stereo pairs are detected with
RaySession's rules (plus FL/FR) and drawn as one group in the matrices; names follow JACK
metadata pretty-names, else RaySession-style graceful names.

The layout (positions, pinned matrices, split/hidden clients, view) is kept in
`~/.config/patchbox/layout.json`.

## Layout of the tree

```
src/                 GUI: main loop (main.rs), app state and actions (app.rs), canvas (canvas.rs),
                     chrome and menus (ui.rs), decorations (csd.rs), KWin shadow (shadow.rs),
                     renderer (gfx.rs) and glyph atlas (text.rs), graph model (model.rs),
                     layout file (layout.rs), scripted test input (testscript.rs)
crates/pbx-jack/     libjack FFI, graph snapshot, events; pbx-fake-clients (test tool)
third_party/         sdl3-src (our CMake wrapper for sdl3-sys), SDL (gitignored, provisioned)
patches/             our SDL changes
tests/nested/        isolated GUI + JACK test harness
docs/                PLAN.md, reference notes on RaySession's patchbay and session manager
assets/fonts/        Cousine (Apache-2.0), PatchMatrix's face
```

## Testing

`tests/nested/run.sh [out.png] [VAR=value…]` runs PatchBox in a nested `kwin_wayland --virtual` on a
private session bus, against a **private** jackd (dummy driver) with fake clients, inside an
unshare'd user+IPC+mount+PID namespace with its own tmpfs `/dev/shm`. The live JACK server and the
desktop are never touched. `PATCHBOX_TEST` scripts input (see `src/testscript.rs`), e.g.

```sh
tests/nested/run.sh /tmp/x.png PATCHBOX_SNAPSHOT_DELAY_MS=600000 \
  "PATCHBOX_TEST=wait 300; click cell:Dragonfly+Room+Reverb>K:system:0:2 shift; wait 300; shot /tmp/a.png; quit"
```

`PATCHBOX_STATS=1` prints main-loop wake-ups and frames at exit (idle: the 1 Hz DSP poll, no
frames).

## Licence

GPL-2.0-or-later. The ideas taken from RaySession (GPL-2.0) and PatchMatrix (Artistic-2.0) are
reimplemented, not copied. Cousine: Apache-2.0 (`assets/fonts/LICENSE`). SDL: zlib.
