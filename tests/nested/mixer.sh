#!/bin/sh
# The settings page's Mixer tab, end to end, without touching a real card: ALSA's
# configuration in the test (ALSA_CONFIG_PATH) puts JackBus's test control plugin (fake-mixer.c,
# "jbtest": an HDA card's Master and Capture, their values in a file) in place of every card's
# control device, so the cards /proc/asound lists open as fake mixers.
#  - the tab lists the cards with audio (a card with MIDI only is not there) and their controls;
#  - dragging Master's bar sets the card's Master (both channels);
#  - its switch mutes it;
#  - a change made by another process (the state file rewritten: an event, as amixer's) shows,
#    and a drag after it keeps the channels' balance;
#  - "Save as my settings" saves the card's state with JackBus's jackbus-mixer, a change made
#    elsewhere after it, then "Back to my settings" puts every value back.
#
#   JACK2_BUILD=/path/to/jack2/build tests/nested/mixer.sh
# (the plugin: make -C nspa/pa-shim test-apps in the jack2 tree)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
jack2=${JACK2_BUILD:?set JACK2_BUILD to a jack2 build directory with pa-shim/libasound_module_ctl_jbtest.so}
plugin=$jack2/pa-shim/libasound_module_ctl_jbtest.so
[ -e "$plugin" ] || { echo "missing $plugin: make -C nspa/pa-shim test-apps"; exit 2; }
W=$HOME/.cache/patchbox-work
st=$W/mixer-state
rm -rf "$st" "$HOME/.cache/patchbox-work/nested/xdg-config/jackbus"; mkdir -p "$st"
rm -f "$W"/mixer-*.png
# every card listed: Master 0 dB both sides, on; Capture +12.75 dB (40), on
for c in $(sed -n 's/^ *\([0-9][0-9]*\) \[.*/\1/p' /proc/asound/cards); do
  echo "86 86 1 1 40 40 1 1" > "$st/$c"
done
first=$(sed -n 's/^ *\([0-9][0-9]*\) \[\([A-Za-z0-9_]*\) *\]: \([^ ]*\) - \(.*\)$/\1/p' /proc/asound/cards | head -1)
cat > "$st/alsa.conf" <<EOF
ctl.hw {
    @args [ CARD ]
    @args.CARD { type string }
    type jbtest
    card \$CARD
    dir "$st"
}
ctl_type.jbtest { lib "$plugin" }
EOF

script=$W/mixer-script.txt
cat > "$script" <<'EOF2'
wait 2500
click bar:settings
wait 1200
click dlg:tab:Mixer
wait 1000
shot HOME/.cache/patchbox-work/mixer-1-tab.png
wait 300
drag dlg:level:Master:98 dlg:level:Master:50
wait 600
click dlg:switch:Master
wait 600
shot HOME/.cache/patchbox-work/mixer-2-muted.png
wait 3000
shot HOME/.cache/patchbox-work/mixer-3-elsewhere.png
wait 300
drag dlg:level:Master:20 dlg:level:Master:75
wait 600
shot HOME/.cache/patchbox-work/mixer-4-again.png
wait 300
click dlg:Save_as_my_settings
wait 2500
click dlg:Back_to_my_settings
wait 800
shot HOME/.cache/patchbox-work/mixer-5-restored.png
wait 300
quit
EOF2
sed -i "s|HOME/|$HOME/|g" "$script"
# Another process's change while the tab is shown, once the drag and the mute are done: the
# state as PatchBox left it kept, then Master rewritten (20 / 30, switches on) in place (the
# plugin watches the file itself, as a card's events reach every process).
(
  for i in $(seq 300); do [ -s "$W/mixer-2-muted.png" ] && break; sleep 0.1; done
  sleep 0.5
  cp "$st/$first" "$st/after-patchbox"
  echo "20 30 1 1 40 40 1 1" > "$st/$first"
  # once the save is there: everything changed elsewhere, before "Back to my settings"
  saved=$HOME/.cache/patchbox-work/nested/xdg-config/jackbus/mixer
  for i in $(seq 300); do ls "$saved"/*.state >/dev/null 2>&1 && break; sleep 0.1; done
  sleep 0.3
  cp "$st/$first" "$st/at-save"
  echo "5 5 0 0 63 63 0 0" > "$st/$first"
) &
timeout 90 "$HERE/run.sh" "$W/mixer-final.png" PB_BIN="$ROOT/target/release" ALSA_CONFIG_PATH="$st/alsa.conf" PATH="$jack2:$PATH" \
  "PATCHBOX_TEST=$(grep -v '^#' "$script" | tr '\n' ';')" PATCHBOX_SNAPSHOT_DELAY_MS=60000 > "$W/mixer-run.log" 2>&1
wait
f() { cut -d' ' -f"$2" "$1" 2>/dev/null; }
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
check "five screenshots" '[ -s "$W/mixer-1-tab.png" ] && [ -s "$W/mixer-2-muted.png" ] && [ -s "$W/mixer-3-elsewhere.png" ] && [ -s "$W/mixer-4-again.png" ] && [ -s "$W/mixer-5-restored.png" ]'
a=$st/after-patchbox
# alsamixer's 50 % on -64.50 .. 0 dB is about -15.9 dB: step 65 or 66 of 86
check "the drag set Master to alsamixer's 50 %, both channels ($(f "$a" 1) $(f "$a" 2))" '[ "$(f "$a" 1)" = "$(f "$a" 2)" ] && [ "$(f "$a" 1)" -ge 63 ] && [ "$(f "$a" 1)" -le 68 ]'
check "its switch muted it ($(f "$a" 3) $(f "$a" 4))" '[ "$(f "$a" 3) $(f "$a" 4)" = "0 0" ]'
z=$st/$first
check "after the change made elsewhere (20 / 30), a drag to 75 % raised both, their balance kept ($(f "$st/at-save" 1) $(f "$st/at-save" 2))" '[ "$(f "$st/at-save" 1)" -gt 20 ] && [ "$(f "$st/at-save" 2)" -gt "$(f "$st/at-save" 1)" ] && [ "$(f "$st/at-save" 2)" -ge 74 ]'
check "Capture untouched ($(f "$st/at-save" 5) $(f "$st/at-save" 6))" '[ "$(f "$st/at-save" 5) $(f "$st/at-save" 6)" = "40 40" ]'
sf=$(ls "$HOME"/.cache/patchbox-work/nested/xdg-config/jackbus/mixer/*.state 2>/dev/null | head -1)
check "saved as my settings: $(basename "${sf:-none}")" '[ -s "$sf" ] && grep -q "^Master|0|pvol|" "$sf"'
check "back to my settings after a change elsewhere ($(cat "$z") against $(cat "$st/at-save"))" '[ "$(cat "$z")" = "$(cat "$st/at-save")" ]'
echo "screenshots: $W/mixer-1-tab.png mixer-2-muted.png mixer-3-elsewhere.png mixer-4-again.png"
exit $fail
