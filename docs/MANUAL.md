# PatchBox manual

Every control, the session daemon, the JACK and PipeWire settings, the source tree and the tests. For what PatchBox is and how to build it, see the [README](../README.md).

## Use

PatchBox connects to the running JACK server (it never starts one) and reconnects when the server
comes back. Its JACK client has no process callback: jack2 starts no RT thread for it and never
puts it in the graph.

| | |
|---|---|
| Title bar | window buttons, ☰ menu (views, scale, grid, undo…), the session's name |
| Toolbar | Luluda-style tiles, an icon over a label: session (New, Open, Save; dropped first when narrow), transport (rewind, play/pause, stop, time and BBT), port type (AUDIO / MIDI / CV / OSC, keys 1–4), view (Ports, its icon showing the flow: click again to switch left-to-right and top-to-bottom (V); Matrix (M)), and the JACK readouts, one LCD ([below](#the-jack-lcd)): DMA (with JackBus), rate and latency, BUF, XRUNS, DSP |
| Views | **Ports** (default): each block shows its ports of the current type, inputs left, outputs right, a wire per connection; **Matrices** (M): PatchMatrix's view, one wire per client pair through a matrix of their ports, port counts on the blocks. Both keep the AUDIO / MIDI / CV / OSC split. Each view (ports left to right, ports top to bottom, matrices) keeps its own block positions and camera, in the global layout and in each session's, so switching views never rearranges another. JACK's graph and PipeWire's each keep their own view: Matrix on PipeWire's leaves JACK's as it was |
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

### The JACK LCD

The JACK readouts are one LCD at the right of the toolbar, each a value over its caption:

| | |
|---|---|
| DMA | with JackBus (a jackdbus with the `cpu-dma-latency` engine parameter; otherwise not shown): the CPU wake-up latency limit JackBus holds while the server runs, in µs, `off` when there is none; dim while the server is stopped (no limit is held then). Click for its levels, read from the computer's own CPU idle states (sysfs cpuidle), from performance to power saving: **0 µs · performance** (no idle state: the CPUs poll, the most power and heat), a row for each idle state but the deepest at its wake-up latency (`1 µs · ≤ C1`, `170 µs · ≤ C6`…: the CPUs may idle down to that state), and **off · power saving** (no limit). A value set elsewhere (the Server tab, a preset) shows as a row of its own. It applies to the running server at once, and jackdbus keeps it for the next start; off resets the parameter. The tooltip says what the value allows (`idle ≤ C1`). PatchBox writes nothing in cpuidle |
| rate | the sample rate in kHz over the latency of one buffer; FREEWHEEL (amber) while freewheeling, `offline` (red) with no server |
| BUF | the buffer size: click for 16 to 4096 frames, changed on the running server |
| XRUNS | xruns since the last reset, with a red bar while there are any: click resets them, right-click opens the event log |
| DSP | the DSP load, with a level (green; amber from 50 %, red from 80 %) |

Each readout is as wide as its widest possible value, so the LCD keeps its size whatever the
values. When the window is narrow, the rate and DMA go before the port types and the transport (the
sidebar shows the rate); the rest of the LCD stays.

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
  open (they ask for its name first). + Application's "Another application" search finds any other
  installed one (a browser, OBS) by name, command or description: type, Enter, pick one of up to 20.
  It is started and stopped with the session (as NSM if its `.desktop` file says so).
- **Each client**: start/stop, save, show/hide its window, × to remove it. Stopping or removing one
  with unsaved changes asks first (save and stop, or stop without saving). Click or right-click its
  name for the rest (kill, properties, keep as a client template, find in the graph).
- **Removed n** brings removed clients back, forgets them, or deletes them with their files.

Nothing is deleted outright: sessions, templates and client files go to the desktop's Trash
(freedesktop spec; Dolphin: Places → Trash), where a file manager can restore them.

After an update, a `patchboxd` still running from before is replaced: at once when no session is
open (by the window, or by itself when no window is connected); with a session open, the session
panel offers **Update** (save, close, restart the daemon, reopen). From a shell:

```sh
patchbox-ctl sessions | new NAME | open NAME | save | close | abort | duplicate NAME | rename NAME | delete NAME
patchbox-ctl add [--plain] EXE [ARGS…] | start ID | stop ID | save-stop ID | kill ID | save-client ID | show-gui ID
patchbox-ctl remove ID [--save] | restore ID | forget ID [--files] | delete-template NAME [--client]
patchbox-ctl connect SRC DST | disconnect SRC DST | graph | state | watch | quit
```

## JACK server

PatchBox is built for JackBus: the server is jackdbus's. The sidebar's JACK · jackdbus
section starts, stops and restarts it, and Settings (a toggle; the arrow before the page's title
goes back to the graph too) edits the driver and engine parameters jackdbus offers, with their
constraints and descriptions, in tabs:

| Tab | |
|---|---|
| Server | state, load, latency and xruns; presets; the engine's parameters; jackdbus itself (Quit jackdbus) |
| Master | the master driver, Switch master (no restart), its parameters |
| Devices | what else joins the graph: sound cards, bridged; then slave drivers |
| Shims | JackBus's shims: PulseAudio, ALSA and PipeWire applications straight to JackBus; PipeWire leaving audio to JackBus ([below](#shims)) |
| PipeWire | PipeWire's settings ([below](#pipewire)) |
| Internals | the other internal clients jackdbus can load |
| Netjack | netmanager, netadapter, and the net drivers as master |

The tabs most used come first; Internals and Netjack, the least used, last. Each tab says what
it is about above its rows, and what applies when in its footer. A session can remember
them (☰ session menu) and puts them back when it opens. Presets (qjackctl's) name a whole
configuration, the driver and every parameter set away from its default: the sidebar's
"Preset ▼" switches between them in one click (the server restarts only when something differs
other than the buffer size and the CPU wake-up latency, which change on the running server)
and the settings page saves and deletes them (`~/.config/patchbox/jack-presets.json`). With
JackBus the ALSA device list follows sound cards plugged in or removed: jackdbus reads it
again and says so, and the page shows the new list.

### Devices: sound cards and slave drivers

**Sound cards, bridged (their own clocks)** needs JackBus. Every card is listed by its id and
name (snd-aloop's reads "a virtual card"): the master's as the master, every other with a switch. On: the card is
bridged now (`AddBridge`) and whenever the server starts or the card is plugged in; its ports
are physical ports, named after the card (`U44:capture_1`), resampled into the master's cycle. A
bridged card shows its state, polled each second while the tab is open: locking or locked, rate,
channels, the delay each way, the card's drift and xruns. Its period, periods and wake-ups
(`auto`, `timer`, `irq`) are set below it; a change reopens that card alone. Off: the bridge is
removed (`RemoveBridge`) and the card's settings are kept (`~/.config/patchbox/bridges-off.json`);
switched on again, it gets them back. A "kept settings" row shows them, and its × forgets them.
A bridge whose card isn't plugged in stays listed, waiting for it, with a × to remove it. The
list is jackdbus's `bridges` engine parameter, kept in its conf.xml; presets and sessions carry
it, and applying it needs no restart. A jackdbus without bridges says so here. A card with MIDI
only (a USB keyboard or controller: no PCM device in `/proc/asound/cardN`) isn't listed, having no
audio to bridge; its MIDI ports come from a MIDI slave driver, below.

**Slave drivers (in the master's cycle)**: alsarawmidi (MIDI ports), alsa (a card clocked by the
master's: word clock, S/PDIF), firewire, net / netone, loopback (JACK's own looped-back ports, not
a card), proxy, dummy. jackdbus takes slave drivers only while stopped, so a running server is
stopped, changed and started again, and the log says so. PatchBox never starts jackd itself; with
a plain jackd the patchbay and transport work, server control doesn't. The transport is in the
toolbar (Space: play/pause).

## Shims

JackBus's shims take applications that speak PulseAudio, ALSA or PipeWire straight to JackBus,
each a JACK client of its own in JackBus's cycle, with no PipeWire in between. They come with the
jackbus package: **jackbus-pa** (a libpulse), **jackbus-alsa** (an ALSA plugin, the `jackbus`
device) and **jackbus-pw** (a libpipewire, for a program started as `jackbus-pw <program>`).
Settings' Shims tab sets them up, a section per kind of application, then PipeWire. Each section's
first row says how its shim is reached and whether it is installed; a setting made for a shim that
isn't installed waits in the file.

**PulseAudio applications**

| | |
|---|---|
| Use JackBus | every PulseAudio application (browsers, players, video calls) started while JackBus runs uses JackBus; one started while it doesn't uses PipeWire, as before. Off by default. From each application's next start, with no new login: one already running stays where it is. Off: applications started afterwards use PipeWire again. `jackbus-pa <program>` runs one program on JackBus whatever the switch says |
| Most a stream buffers | the most a PulseAudio stream may queue in the shim, beyond the device's latency: what it asks for (the default), 10 to 160 ms, or a number typed in. Browsers ask for 30–50 ms, players more; lower is tighter, too low drops out |

**ALSA applications**

| | |
|---|---|
| Default device is JackBus | every ALSA application's `default` device is JackBus, from the application's next start. Off: `default` is as the system has it again (PipeWire's, with pipewire-alsa) |
| While JackBus isn't running | where `default` goes then: PipeWire, the sound card, or nothing (no sound). Until chosen: PipeWire when pipewire-alsa is installed, else the card |

**PipeWire applications** go to JackBus when started as `jackbus-pw <program>`.

**Each kind of application** has these, its own:

| | |
|---|---|
| Resample another rate | on (the default): a stream at JackBus's rate goes as it is, one at another rate is resampled; off: a stream at another rate is refused (PipeWire and PulseAudio programs) or held to JackBus's rate (ALSA) |
| Resampling quality | JackBus's bridge resampler: low (32 taps, the least CPU), normal (56, the default), high (96: the most CPU, a few frames more delay) |
| Connect new streams | on (the default): a new stream's ports are connected to the output (input) device; off: left unconnected, for you or a session to route |
| Output device, Input device | where new playback (recording) streams go: the first card (`system`, the default) or another JACK client with physical ports, such as a bridged card; one chosen earlier that isn't there now is marked so. For PulseAudio applications, also their default output and input |

These apply to new streams.

**PipeWire**

| | |
|---|---|
| Leaves audio to JackBus | PipeWire's JACK bridge isn't loaded. While JackBus runs, PipeWire has no audio work: JackBus has the cards, the shims the PulseAudio and ALSA applications. While it doesn't, PipeWire plays as before. Video and screen sharing are untouched; applications only PipeWire plays (Flatpaks, for instance) are silent while JackBus runs. Applies when PipeWire restarts (Restart PipeWire, or the next login); off, the bridge is back at the next restart |
| Restart PipeWire | applies the switch above now: `systemctl --user restart pipewire.service` (WirePlumber restarts with it; pipewire-pulse stops and starts again for the next program; a screen share in progress stops) |

What the tab writes (under `$XDG_CONFIG_HOME` when it is set); turning a switch off takes back
what it wrote, and nothing else is touched:

- `~/.config/jackbus/compat.conf`, the shims' settings file, `key = value`: the tab writes each
  shim's own key (`pa.resample`, `alsa.output`, `pw.autoconnect`…), `pa.max-latency`, `pulse`
  (Use JackBus) and `alsa.fallback`, and keeps the file's other lines and comments. A shim reads
  its own key first, then the shared one (`resample = no`, written by hand, applies to every shim
  without a key of its own), in this file and then in `/etc/jackbus/compat.conf`.
- `~/.config/alsa/asoundrc`: a block between two `# PatchBox: …` lines,
  `pcm.!default { type jackbus fallback "pipewire" }` (`sysdefault` for the card, no `fallback`
  for nothing). Your own lines stay; off removes the block, and the file if nothing else is in it.
- `~/.config/pipewire/pipewire.conf.d/60-patchbox-no-jack-bridge.conf`:
  `module.jackdbus-detect = false`. Off deletes it.

The first version of Use JackBus wrote `~/.config/environment.d/60-jackbus-pa.conf` (a library
path, from the next login); PatchBox turns it into `pulse = yes` when it starts, and removes it.

## PipeWire

Settings' PipeWire tab is read from `pw-dump --monitor` and written with
`pw-metadata` / `pw-cli`. It adapts to what runs:

- **On top of JACK** (JackBus with module-jackdbus-detect, or module-jack-tunnel): the
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
                     (server.rs), sound cards (bridges.rs), CPU idle states for the DMA menu
                     (cpuidle.rs), PipeWire's graph and settings (pw.rs, pwconf.rs), JackBus's
                     shims' settings (compat.rs)
crates/pbx-jack/     libjack FFI, graph snapshot, events, transport; pbx-fake-clients (test tool)
crates/pbx-session/  patchboxd (NSM server, clients, connection memory), patchbox-ctl, the
                     session format and control protocol; pbx-fake-nsm (test tool)
third_party/         sdl3-src (our CMake wrapper for sdl3-sys), SDL (gitignored, provisioned)
patches/             our SDL changes
tests/nested/        isolated GUI + JACK test harness
docs/                MANUAL.md (this file), index.html (the architecture document)
assets/fonts/        Cousine (Apache-2.0), PatchMatrix's face
```

## The static SDL

SDL 3 is built for PatchBox alone and linked statically (the `vendored-sdl` feature, on by default;
`--no-default-features` links the system's `libSDL3.so` instead).

| | |
|---|---|
| Source | `third_party/SDL`: upstream `release-3.4.18` (`829a65d7`), gitignored; `scripts/sdl.sh fetch` provisions it |
| Changes | `patches/sdl-patchbox.patch` (`scripts/sdl.sh regen` rewrites it from the tree) |
| Configuration | `third_party/sdl3-src/cmake/CMakeLists.txt` |
| Build | `cargo build`; `sdl3-sys` is pinned `=0.7.2` (it builds SDL 3.4.18; 0.7.3 would fetch crates.io's 3.4.20 instead) |
| Check | `ldd target/release/patchbox` shows no `libSDL3`; the binary carries `SDL-release-3.4.18-829a65d7 (patchbox)` |

What's in it: video on Wayland only, OpenGL through EGL. What isn't: SDL's renderer and GPU API,
audio (JACK is PatchBox's), camera, joystick, haptic, HIDAPI, sensor, power, dialog and tray,
libdecor (`csd.rs` draws the decorations), X11, KMS, Vulkan, GLES and D-Bus. Without D-Bus the
compositor draws the cursors (cursor-shape protocol), text input goes through text-input-v3, and
PatchBox lets the screensaver be (`SDL_VIDEO_ALLOW_SCREENSAVER=1`); it uses neither the
colour-scheme portal nor `SDL_OpenURL`, so libdbus isn't loaded. The C objects are compiled with
`-ffunction-sections -fdata-sections`, so the linker drops the functions PatchBox never reaches.
GL comes from GLVND's `libOpenGL.so.0` (`SDL_OPENGL_LIBRARY`), without GLX and Xlib; SDL falls back
to `libGL.so.1` when it's missing.

The patch:

1. Dynamic API off (no jump table, no `SDL3_DYNAMIC_API` override).
2. Wake-ups through an eventfd polled next to the display fd, not a `wl_display_sync` round trip
   through the compositor. JACK's notification thread wakes the main loop this way.
3. Lost wake-up (upstream bug): an event pushed between the wait loop's last peek and its storing
   `wakeup_window` sent no wake-up. It matters here: the loop waits up to a second when idle.
4. Key repeat after a preemption (upstream bug, unsigned wrap).
5. Clipboard offer kept across focus moves between the program's own windows (3.4.18 regression).
6. No cursor animation thread (it made every main-thread read wait for it).
7. No colour-management binding (synchronous round trips in event dispatch, never used).
8. A scroll in the same pointer frame as a leave (upstream NULL dereference).

## Testing

`tests/nested/run.sh [out.png] [VAR=value…]` runs PatchBox in a nested `kwin_wayland --virtual` on a
private session bus, against a **private** jackd (dummy driver) with fake clients, inside an
unshare'd user+IPC+mount+PID namespace with its own tmpfs `/dev/shm`. The live JACK server and the
desktop are never touched. `PATCHBOX_TEST` scripts input (see `src/testscript.rs`), e.g.

```sh
tests/nested/run.sh /tmp/x.png PATCHBOX_SNAPSHOT_DELAY_MS=600000 \
  "PATCHBOX_TEST=wait 300; click cell:Dragonfly+Room+Reverb>K:system:0:2 shift; wait 300; shot /tmp/a.png; quit"
```

`tests/nested/cameras.sh` checks that each graph (JACK, PipeWire) and each session canvas keeps,
per view style (Ports, vertical Ports, Matrix), its own camera and block/matrix positions across
style and graph switches, Fit, sessions (opened or closed while on PipeWire) and restarts (scripts
in `tests/nested/cameras/`; every `cam =X` must equal `cam X`). It fails on the code before the fix
it came with.

With a JackBus build (`JACK2_BUILD=/path/to/jack2/build`), three more drive a JackBus jackdbus on
the harness's private bus: `tests/nested/bridges.sh` (snd-aloop's card bridged from the Devices
tab, its period changed, switched off and on), `tests/nested/hotplug.sh` (a card's `/dev/snd`
nodes taken away and brought back inside the namespace) and `tests/nested/cpuwake.sh` (the DMA
readout: a level held, then off).

`tests/session/run.sh` drives patchboxd end to end (sessions, an NSM client, connection memory)
against a private jackd in its own namespace. `PB_OLD_DAEMON=<patchboxd>` (with `PB_OLD_SETUP`)
starts the nested harness against an older daemon already running; `PB_KEEP_CFG=1` keeps the last
run's PatchBox config. `PB_JACKDBUS=1` makes the nested harness run
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
