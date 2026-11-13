# PatchBox

A JACK patchbay — and, next, session manager — for one Linux audio box: the graph of PatchMatrix
(client nodes with connection matrices between them, the owner's high-contrast colours), with
RaySession's flexibility underneath and its session management beside it. Rust, SDL3 (vendored,
static, Wayland-only) and GL drawn by the app itself. No Qt.

Status: the live patchbay, JACK server control (jackdbus), transport and the session manager
(`patchboxd`, an NSM server) work; see `docs/PLAN.md` for what comes next.

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
| Title bar | window buttons, ☰ menu (views, scale, grid, undo…), the session's name |
| Toolbar | Luluda-style tiles, an icon over a label: session (New, Open, Save; dropped first when narrow), transport (rewind, play/pause, stop, time and BBT), port type (AUDIO / MIDI / CV / OSC, keys 1–4), view (Ports, its icon showing the flow: click again to switch left-to-right and top-to-bottom (V); Matrix (M)), JACK readouts (rate and latency, buffer size menu, xruns: click to reset, right-click for the event log, DSP load) |
| Views | **Ports** (default): each block shows its ports of the current type, inputs left, outputs right, a wire per connection; **Matrices** (M): PatchMatrix's view, one wire per client pair through a matrix of their ports, port counts on the blocks. Both keep the AUDIO / MIDI / CV / OSC split. Each view (ports left to right, ports top to bottom, matrices) keeps its own block positions and camera, in the global layout and in each session's, so switching views never rearranges another |
| Tracks | right-click a block whose ports name tracks or plugins (Ardour's `Track/port`, Carla's `Plugin:port`) → A block per track / plugin: each becomes its own block, titled after it, the client above it; the rest stay in the client's block. Right-click any of them → One block for the whole client |
| Port groups | right-click a port → Group ports from here…: a name and how many (this port and the ones after it). A bracket joins them; dragging one connects the whole group 1:1 to the group (or the ports from) where it lands, or disconnects them all when they already are; Ctrl+drag: that port alone. A group of two is a stereo pair (Shift-painting in matrices). Kept with the layout (per session in a session) |
| Names | right-click a block or port → Rename: your names show here and, unless ☰ → Export our names is off, in other JACK programs too (JACK metadata pretty-name, RaySession's rules: one exporter at a time, someone else's pretty-name never overwritten, new ports left 200 ms, ours withdrawn when a name is cleared and when PatchBox quits; a split block's name stays ours) |
| Find | Ctrl+F (or ☰ → Find): type part of a block's or a port's name (JACK's, pretty, alias, or yours); Up/Down and Enter, or a click, centres it (a port: its type shown, a ring round it); Escape closes |
| Selection | Ctrl+drag on empty canvas: select the blocks it touches (adds to the selection); Ctrl+click a block: add / remove it; Ctrl+A: all; Escape or a click on empty canvas: none. Dragging a selected block moves them all; right-click one: align edges, centres, tops, middles, bottoms, space evenly across or down, hide them |
| Port | drag onto another port: connect / disconnect; onto a block: its first free compatible port; hover: full name, aliases, latency (capture and playback ranges, frames and ms), connections |
| Wire (ports view) | hover: lights up with its name; drag it by an end: drop on another port or block to move the connection there, on empty canvas to disconnect; right-click: disconnect |
| Matrix cell | click: connect / disconnect; drag across cells: paint the same state; **Shift**: the stereo partner cell too |
| Node | drag: move (its matrices follow halfway); double-click (or E for all): list **all** its ports, every type, colour-coded; right-click: show ports, split/join inputs and outputs, disconnect, hide |
| Port row (expanded node) | hover: full name, type, aliases, connections; drag onto another port: connect / disconnect; drag onto a node: connect to its first free compatible port |
| Output handle → node | drag from a node's right handle (or Shift+drag the node) onto another node: open a matrix between them; hold **Ctrl** at the drop to connect 1:1 |
| Matrix | Ctrl+drag: move; right-click: connect 1:1, disconnect all, remove. Its wires meet at its centre; pull one off its block: onto another block, the connections move there port for port; onto empty canvas, they are disconnected |
| Canvas | G: grid on/off; Shift+G: snap blocks to the grid; drag or middle-drag: pan; wheel: scroll; zoom: the widget at the bottom right (centre button: fit the graph, −, slider, +, click the % for 100 %), Ctrl+wheel, touchpad pinch, Ctrl+= / Ctrl+− / Ctrl+0; F: fit; L: port names on every matrix (hovering shows a matrix's names anyway); F9: sidebar |
| Connection rules | ☰ → Connection rules, or right-click a wire (Always keep it) or a block (Always keep its connections): patchboxd makes these connections whenever both ports are there, session or not (it stays running while there are rules); a name may hold `*` (`PipeWire*:capture_FL`); a connection removed by hand stays removed until one of its ports comes back; sessions never undo them; `patchbox-ctl rules / rule-add / rule-del` |
| Event log | right-click XRUNS (a click resets the count), or click the sidebar's log, or ☰ → Event log: xruns with the time, how late and the DSP load, the server's changes, clients, connections, PatchBox's messages (filter: Show); reset the xrun count here and the server's; a second tab shows the end of jackdbus's own log |
| Interface scale | ☰ menu → Interface scale (default 125 %), or Ctrl+Shift+= / Ctrl+Shift+−: everything in the window except the window buttons, which follow the desktop's scale |
| Sidebar | clients (click: find; right-click: menu, show hidden ones), log |
| Client templates | a session client's menu → Save as a client template…: its command, properties and files (those named after it in the session folder); Add an application lists your templates first, and a client added from one gets its own copy of the files. `patchbox-ctl client-templates / save-client-template ID NAME / add-template NAME` |
| Client hooks | `patchbox-scripts/clients/<id>/{start,save,stop}.sh`, else `patchbox-scripts/clients/{start,save,stop}.sh`, in the session folder or one above it: run beside the client when it starts, after it saved, after it stopped (they don't hold it up), with `PATCHBOX_CLIENT_ID`, `PATCHBOX_CLIENT_PID`, `PATCHBOX_CLIENT_JACK_NAME` and the session's variables |
| Session client | right-click it in the session panel → Properties…: label, command (quoted words; `$SESSION_NAME`, `$SESSION_PATH`, `$CLIENT_ID`, `$JACK_CLIENT_NAME` become theirs at start), environment (`NAME=value …`), run in a terminal; a plain (non-NSM) program's save signal (counts as saved after 300 ms) and stop signal. From its next start. Also `patchbox-ctl props ID save=USR1 env=K=V … -- CMD ARGS` |

Physical ports and a2j devices start split into a capture node and a playback node (PatchMatrix's
and RaySession's default); any client can be split or joined. Stereo pairs are detected with
RaySession's rules (plus FL/FR) and drawn as one group in the matrices; names follow JACK
metadata pretty-names, else RaySession-style graceful names.

The layout (positions, pinned matrices, split/hidden clients, view) is kept in
`~/.config/patchbox/layout.json`.

## Sessions

`patchboxd` runs the session: an NSM server (Ardour, Carla, Hydrogen… are told to open and save),
plain programs started and stopped with it, and the session's JACK connections, restored as ports
appear (RaySession's rules: a connection you removed stays removed; an application auto-connecting
against the session is undone). The GUI starts it when needed; it keeps running the session if the
GUI quits, and exits when idle with no session.

A session is a folder under `~/PatchBox Sessions` (`PATCHBOX_SESSIONS`): `session.toml` (our
format: clients, trash, notes, connections), `patchbox-layout.json` (the canvas as you left it),
each NSM client's files in `<name>.<id>/`, client logs in `.patchbox/`.

From the sidebar (hover any icon for what it does):

- **Sessions**: New / Open / Save / Close; Open lists them, × deletes one (asks first). ⋯ has
  duplicate, rename, notes, templates (use or delete), snapshots, open the folder, close without
  saving, delete this session (both ask first).
- **Programs**: + Application (NSM-capable `.desktop` entries, other audio applications, your
  client templates) and Executable (any command line), as in RaySession; both work with no session
  open (they ask for its name first).
- **Each client**: start/stop, save, show/hide its window, × to remove it. Stopping or removing one
  with unsaved changes asks first (save and stop, or stop without saving). Click or right-click its
  name for the rest (kill, properties, keep as a client template, find in the graph).
- **Removed n** brings removed clients back, forgets them, or deletes them with their files.

Nothing is deleted outright: sessions, templates and client files go to the desktop's Trash
(freedesktop spec), where a file manager can restore them. From a shell:

```sh
patchbox-ctl sessions | new NAME | open NAME | save | close | abort | duplicate NAME | rename NAME | delete NAME
patchbox-ctl add [--plain] EXE [ARGS…] | start ID | stop ID | save-stop ID | kill ID | save-client ID | show-gui ID
patchbox-ctl remove ID [--save] | restore ID | forget ID [--files] | delete-template NAME [--client]
patchbox-ctl connect SRC DST | disconnect SRC DST | graph | state | watch | quit
```

## JACK server

PatchBox is built for jack2dbus-nspa: the server is jackdbus's. The sidebar's JACK · jackdbus
section starts, stops and restarts it, and Settings (a toggle) edits the driver and engine
parameters jackdbus offers, with their constraints and descriptions. A session can remember
them (☰ session menu) and puts them back when it opens. Presets (qjackctl's) name a whole
configuration, the driver and every parameter set away from its default: the sidebar's
"Preset ▼" switches between them in one click (the server restarts only when something differs)
and the settings page saves and deletes them (`~/.config/patchbox/jack-presets.json`). PatchBox never starts jackd itself; with
a plain jackd the patchbay and transport work, server control doesn't. The transport is in the
toolbar (Space: play/pause).

## PipeWire

Settings has a second tab, PipeWire, read from `pw-dump --monitor` and written with
`pw-metadata` / `pw-cli`. It adapts to what runs:

- **On top of JACK** (jack2dbus-nspa with module-jackdbus-detect, or module-jack-tunnel): the
  bridge drives PipeWire's graph, so its clock follows JACK's and is shown, not edited; the
  bridge's nodes and ports, and the file that configures the module (its arguments apply when
  PipeWire restarts).
- **Alone** (PipeWire as the JACK server too, through pipewire-jack, or beside one): the clock
  is editable live (default rate, allowed rates, quanta; forced rate and quantum until PipeWire
  restarts), and the defaults can be kept in `~/.config/pipewire/pipewire.conf.d/60-patchbox-clock.conf`.
  Without jackdbus, Settings opens on this tab.

Either way: the log level, the default output and input (marking a choice the session manager
can't use, e.g. a sink chosen as the input), device profiles, WirePlumber's settings from its
own schema, and a PipeWire restart. PipeWire's graph itself opens from its JACK client (double
click), never mixed into the JACK graph.

## Layout of the tree

```
src/                 GUI: main loop (main.rs), app state and actions (app.rs), canvas (canvas.rs),
                     chrome and menus (ui.rs), decorations (csd.rs), KWin shadow (shadow.rs),
                     renderer (gfx.rs) and glyph atlas (text.rs), graph model (model.rs),
                     layout file (layout.rs), scripted test input (testscript.rs), jackdbus
                     (server.rs), PipeWire's graph and settings (pw.rs, pwconf.rs)
crates/pbx-jack/     libjack FFI, graph snapshot, events, transport; pbx-fake-clients (test tool)
crates/pbx-session/  patchboxd (NSM server, clients, connection memory), patchbox-ctl, the
                     session format and control protocol; pbx-fake-nsm (test tool)
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

`tests/session/run.sh` drives patchboxd end to end (sessions, an NSM client, connection memory)
against a private jackd in its own namespace. `PB_JACKDBUS=1` makes the nested harness run
jackdbus (scratch HOME) instead of jackd; `PB_PIPEWIRE=1` adds a private PipeWire beside it,
`PB_PIPEWIRE=bridge` one on top of it (module-jack-tunnel, with stand-in session manager
metadata). PatchBox gets a scratch XDG config home there.

Sanitizers (nightly Rust): `scripts/sanitize.sh address` (or `thread`) builds into
`target/<sanitizer>/`; `PB_BIN=$(scripts/sanitize.sh address --bin-dir)` runs the session tests and
the GUI harness against it (`ASAN_OPTIONS=detect_odr_violation=0` for the GUI: SDL's stb_image and
miniz hold the same 19-byte table, which the linker merges). ThreadSanitizer:
`TSAN_OPTIONS=suppressions=$PWD/scripts/tsan.supp` (a lock-order warning inside libdb, on our
main thread only).

`PATCHBOX_STATS=1` prints main-loop wake-ups and frames at exit (idle: the 1 Hz DSP poll, no
frames).

## Licence

GPL-2.0-or-later. The ideas taken from RaySession (GPL-2.0) and PatchMatrix (Artistic-2.0) are
reimplemented, not copied. Cousine: Apache-2.0 (`assets/fonts/LICENSE`). SDL: zlib.
