//! The sound cards' own mixers: the settings page's Mixer tab. Every control of each card with
//! audio, as alsamixer shows them, through alsa-lib's simple mixer: a volume as alsamixer's bar
//! (its scale over the control's dB range) with the control's switch beside it, a switch on its
//! own, a list (an input source, an auto-mute mode). A card with MIDI only has no audio and no
//! row; snd-aloop's virtual card is left out; PipeWire isn't a card.
//!
//! The mixers are open while the tab is shown. Their events (a change made anywhere: here,
//! alsamixer, a desktop's volume control, JackBus's jackbus-pa) are read on the main loop's
//! tick, and the list of cards is read again every second: a card plugged in appears, one
//! unplugged goes.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_void};

#[repr(C)]
struct SndMixer {
    _p: [u8; 0],
}

#[repr(C)]
struct SndElem {
    _p: [u8; 0],
}

/// SND_CTL_TLV_DB_GAIN_MUTE: a dB range whose minimum is silence
const GAIN_MUTE: c_long = -9_999_999;
/// alsamixer: a control whose range is 24 dB or less moves linearly in dB
const LINEAR_DB: c_long = 2400;
/// SND_MIXER_SCHN_LAST + 1
const CHANNELS: c_int = 32;

#[link(name = "asound")]
unsafe extern "C" {
    fn snd_mixer_open(m: *mut *mut SndMixer, mode: c_int) -> c_int;
    fn snd_mixer_attach(m: *mut SndMixer, name: *const c_char) -> c_int;
    fn snd_mixer_selem_register(m: *mut SndMixer, options: *mut c_void, classp: *mut *mut c_void) -> c_int;
    fn snd_mixer_load(m: *mut SndMixer) -> c_int;
    fn snd_mixer_close(m: *mut SndMixer) -> c_int;
    fn snd_mixer_handle_events(m: *mut SndMixer) -> c_int;
    fn snd_mixer_poll_descriptors_count(m: *mut SndMixer) -> c_int;
    fn snd_mixer_poll_descriptors(m: *mut SndMixer, pfds: *mut libc::pollfd, space: c_uint) -> c_int;
    fn snd_mixer_first_elem(m: *mut SndMixer) -> *mut SndElem;
    fn snd_mixer_elem_next(e: *mut SndElem) -> *mut SndElem;

    fn snd_mixer_selem_get_name(e: *mut SndElem) -> *const c_char;
    fn snd_mixer_selem_get_index(e: *mut SndElem) -> c_uint;
    fn snd_mixer_selem_is_active(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_common_volume(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_playback_volume(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_capture_volume(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_common_switch(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_playback_switch(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_capture_switch(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_has_playback_channel(e: *mut SndElem, ch: c_int) -> c_int;
    fn snd_mixer_selem_has_capture_channel(e: *mut SndElem, ch: c_int) -> c_int;

    fn snd_mixer_selem_get_playback_volume_range(e: *mut SndElem, min: *mut c_long, max: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_capture_volume_range(e: *mut SndElem, min: *mut c_long, max: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_playback_dB_range(e: *mut SndElem, min: *mut c_long, max: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_capture_dB_range(e: *mut SndElem, min: *mut c_long, max: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_playback_volume(e: *mut SndElem, ch: c_int, v: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_capture_volume(e: *mut SndElem, ch: c_int, v: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_playback_dB(e: *mut SndElem, ch: c_int, v: *mut c_long) -> c_int;
    fn snd_mixer_selem_get_capture_dB(e: *mut SndElem, ch: c_int, v: *mut c_long) -> c_int;
    fn snd_mixer_selem_set_playback_volume(e: *mut SndElem, ch: c_int, v: c_long) -> c_int;
    fn snd_mixer_selem_set_capture_volume(e: *mut SndElem, ch: c_int, v: c_long) -> c_int;
    fn snd_mixer_selem_set_playback_dB(e: *mut SndElem, ch: c_int, v: c_long, dir: c_int) -> c_int;
    fn snd_mixer_selem_set_capture_dB(e: *mut SndElem, ch: c_int, v: c_long, dir: c_int) -> c_int;
    fn snd_mixer_selem_get_playback_switch(e: *mut SndElem, ch: c_int, v: *mut c_int) -> c_int;
    fn snd_mixer_selem_get_capture_switch(e: *mut SndElem, ch: c_int, v: *mut c_int) -> c_int;
    fn snd_mixer_selem_set_playback_switch_all(e: *mut SndElem, v: c_int) -> c_int;
    fn snd_mixer_selem_set_capture_switch_all(e: *mut SndElem, v: c_int) -> c_int;

    fn snd_mixer_selem_is_enumerated(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_get_enum_items(e: *mut SndElem) -> c_int;
    fn snd_mixer_selem_get_enum_item_name(e: *mut SndElem, idx: c_uint, maxlen: usize, s: *mut c_char) -> c_int;
    fn snd_mixer_selem_get_enum_item(e: *mut SndElem, ch: c_int, idx: *mut c_uint) -> c_int;
    fn snd_mixer_selem_set_enum_item(e: *mut SndElem, ch: c_int, idx: c_uint) -> c_int;
}

/// A control, as the tab shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A volume: alsamixer's position (0..1), the text beside it (dB, its percentage), and the
    /// control's switch if it has one (on: plays or records).
    Level { pos: f32, text: String, switch: Option<bool>, help: String },
    Switch(bool),
    /// The items and the one chosen.
    List(Vec<String>, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// "<card index>:<p|c|e>:<element index>:<name>": what a change names.
    pub key: String,
    pub label: String,
    pub value: Value,
}

/// A card: its controls, or why its mixer didn't open.
#[derive(Clone, Debug, PartialEq)]
pub struct CardView {
    pub index: u32,
    pub id: String,
    pub name: String,
    pub controls: Result<Vec<Control>, String>,
}

struct Card {
    card: crate::bridges::Card,
    mixer: *mut SndMixer,
    error: Option<String>,
}

impl Drop for Card {
    fn drop(&mut self) {
        if !self.mixer.is_null() {
            unsafe { snd_mixer_close(self.mixer) };
        }
    }
}

impl Card {
    fn open(card: crate::bridges::Card) -> Card {
        let mut m: *mut SndMixer = std::ptr::null_mut();
        let name = CString::new(format!("hw:{}", card.index)).unwrap_or_default();
        let err = unsafe {
            let mut r = snd_mixer_open(&mut m, 0);
            if r >= 0 {
                r = snd_mixer_attach(m, name.as_ptr());
            }
            if r >= 0 {
                r = snd_mixer_selem_register(m, std::ptr::null_mut(), std::ptr::null_mut());
            }
            if r >= 0 {
                r = snd_mixer_load(m);
            }
            r
        };
        if err < 0 {
            if !m.is_null() {
                unsafe { snd_mixer_close(m) };
            }
            let msg = unsafe { CStr::from_ptr(libc::strerror(-err)) }.to_string_lossy().into_owned();
            return Card { card, mixer: std::ptr::null_mut(), error: Some(msg) };
        }
        Card { card, mixer: m, error: None }
    }

    fn elems(&self) -> impl Iterator<Item = *mut SndElem> {
        let mut e = if self.mixer.is_null() { std::ptr::null_mut() } else { unsafe { snd_mixer_first_elem(self.mixer) } };
        std::iter::from_fn(move || {
            if e.is_null() {
                return None;
            }
            let cur = e;
            e = unsafe { snd_mixer_elem_next(e) };
            Some(cur)
        })
    }

    fn find(&self, index: u32, name: &str) -> Option<*mut SndElem> {
        self.elems().find(|&e| unsafe { snd_mixer_selem_get_index(e) } == index && elem_name(e) == name)
    }

    fn view(&self) -> CardView {
        let c = &self.card;
        let controls = match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(self.elems().flat_map(|e| controls_of(c.index, e)).collect()),
        };
        CardView { index: c.index, id: c.id.clone(), name: c.name.clone(), controls }
    }
}

fn elem_name(e: *mut SndElem) -> String {
    let p = unsafe { snd_mixer_selem_get_name(e) };
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// One direction of an element: playback (also a "common" control's) or capture.
#[derive(Clone, Copy)]
struct Dir(bool);

impl Dir {
    fn has_volume(self, e: *mut SndElem) -> bool {
        unsafe {
            if self.0 {
                snd_mixer_selem_has_playback_volume(e) != 0 || snd_mixer_selem_has_common_volume(e) != 0
            } else {
                snd_mixer_selem_has_capture_volume(e) != 0 && snd_mixer_selem_has_common_volume(e) == 0
            }
        }
    }
    fn has_switch(self, e: *mut SndElem) -> bool {
        unsafe {
            if self.0 {
                snd_mixer_selem_has_playback_switch(e) != 0 || snd_mixer_selem_has_common_switch(e) != 0
            } else {
                snd_mixer_selem_has_capture_switch(e) != 0 && snd_mixer_selem_has_common_switch(e) == 0
            }
        }
    }
    fn channels(self, e: *mut SndElem) -> Vec<c_int> {
        (0..CHANNELS)
            .filter(|&ch| unsafe { if self.0 { snd_mixer_selem_has_playback_channel(e, ch) } else { snd_mixer_selem_has_capture_channel(e, ch) } } != 0)
            .collect()
    }
    fn raw_range(self, e: *mut SndElem) -> Option<(c_long, c_long)> {
        let (mut a, mut b) = (0, 0);
        let r = unsafe { if self.0 { snd_mixer_selem_get_playback_volume_range(e, &mut a, &mut b) } else { snd_mixer_selem_get_capture_volume_range(e, &mut a, &mut b) } };
        (r >= 0 && a < b).then_some((a, b))
    }
    fn db_range(self, e: *mut SndElem) -> Option<(c_long, c_long)> {
        let (mut a, mut b) = (0, 0);
        let r = unsafe { if self.0 { snd_mixer_selem_get_playback_dB_range(e, &mut a, &mut b) } else { snd_mixer_selem_get_capture_dB_range(e, &mut a, &mut b) } };
        (r >= 0 && a < b).then_some((a, b))
    }
    fn raw(self, e: *mut SndElem, ch: c_int) -> Option<c_long> {
        let mut v = 0;
        let r = unsafe { if self.0 { snd_mixer_selem_get_playback_volume(e, ch, &mut v) } else { snd_mixer_selem_get_capture_volume(e, ch, &mut v) } };
        (r >= 0).then_some(v)
    }
    fn db(self, e: *mut SndElem, ch: c_int) -> Option<c_long> {
        let mut v = 0;
        let r = unsafe { if self.0 { snd_mixer_selem_get_playback_dB(e, ch, &mut v) } else { snd_mixer_selem_get_capture_dB(e, ch, &mut v) } };
        (r >= 0).then_some(v)
    }
    fn switch(self, e: *mut SndElem, ch: c_int) -> Option<bool> {
        let mut v = 0;
        let r = unsafe { if self.0 { snd_mixer_selem_get_playback_switch(e, ch, &mut v) } else { snd_mixer_selem_get_capture_switch(e, ch, &mut v) } };
        (r >= 0).then_some(v != 0)
    }

    /// alsamixer's position of a channel (0..1)
    fn norm(self, e: *mut SndElem, ch: c_int) -> Option<f64> {
        let Some((min, max)) = self.db_range(e) else {
            let (min, max) = self.raw_range(e)?;
            return Some((self.raw(e, ch)? - min) as f64 / (max - min) as f64);
        };
        let v = self.db(e, ch)?;
        if max - min <= LINEAR_DB {
            return Some((v - min) as f64 / (max - min) as f64);
        }
        let mut n = 10f64.powf((v - max) as f64 / 6000.0);
        if min != GAIN_MUTE {
            let nmin = 10f64.powf((min - max) as f64 / 6000.0);
            n = (n - nmin) / (1.0 - nmin);
        }
        Some(n.clamp(0.0, 1.0))
    }

    /// Set a channel to alsamixer's position `n`, rounded the way it moves (`dir`), so a small
    /// step still moves the control
    fn set_norm(self, e: *mut SndElem, ch: c_int, n: f64, dir: c_int) -> bool {
        let n = n.clamp(0.0, 1.0);
        let round = |x: f64| if dir > 0 { x.ceil() } else if dir < 0 { x.floor() } else { x.round() } as c_long;
        let set_raw = |v: c_long| unsafe { if self.0 { snd_mixer_selem_set_playback_volume(e, ch, v) } else { snd_mixer_selem_set_capture_volume(e, ch, v) } } >= 0;
        let set_db = |v: c_long| unsafe { if self.0 { snd_mixer_selem_set_playback_dB(e, ch, v, dir) } else { snd_mixer_selem_set_capture_dB(e, ch, v, dir) } } >= 0;
        let Some((min, max)) = self.db_range(e) else {
            let Some((rmin, rmax)) = self.raw_range(e) else { return false };
            return set_raw(rmin + round(n * (rmax - rmin) as f64));
        };
        if n <= 0.0 {
            return self.raw_range(e).is_some_and(|(rmin, _)| set_raw(rmin));
        }
        if n >= 1.0 {
            return set_db(max);
        }
        if max - min <= LINEAR_DB {
            return set_db(min + round(n * (max - min) as f64));
        }
        let mut n = n;
        if min != GAIN_MUTE {
            let nmin = 10f64.powf((min - max) as f64 / 6000.0);
            n = n * (1.0 - nmin) + nmin;
        }
        set_db(max + round(6000.0 * n.log10()))
    }
}

fn db_text(v: c_long) -> String {
    if v <= GAIN_MUTE {
        return "−∞ dB".into();
    }
    format!("{:.2} dB", v as f64 / 100.0).replace('-', "−")
}

/// An element's rows: a volume (with its switch) per direction it has, a switch with no volume,
/// or a list.
fn controls_of(card: u32, e: *mut SndElem) -> Vec<Control> {
    let mut out = Vec::new();
    if unsafe { snd_mixer_selem_is_active(e) } == 0 {
        return out;
    }
    let name = elem_name(e);
    let index = unsafe { snd_mixer_selem_get_index(e) };
    let base = if index > 0 { format!("{name} {index}") } else { name.clone() };
    if unsafe { snd_mixer_selem_is_enumerated(e) } != 0 {
        let n = unsafe { snd_mixer_selem_get_enum_items(e) }.max(0) as u32;
        let items: Vec<String> = (0..n)
            .map(|i| {
                let mut buf = [0 as c_char; 64];
                unsafe { snd_mixer_selem_get_enum_item_name(e, i, buf.len(), buf.as_mut_ptr()) };
                unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
            })
            .collect();
        let mut cur = 0;
        unsafe { snd_mixer_selem_get_enum_item(e, 0, &mut cur) };
        out.push(Control { key: format!("{card}:e:{index}:{name}"), label: base, value: Value::List(items, cur as usize) });
        return out;
    }
    let both = (Dir(true).has_volume(e) || Dir(true).has_switch(e)) && (Dir(false).has_volume(e) || Dir(false).has_switch(e));
    for d in [Dir(true), Dir(false)] {
        let label = if both && !d.0 { format!("{base} · capture") } else { base.clone() };
        let key = format!("{card}:{}:{index}:{name}", if d.0 { 'p' } else { 'c' });
        let chans = d.channels(e);
        let switch = if d.has_switch(e) { chans.iter().find_map(|&ch| d.switch(e, ch)).or_else(|| d.switch(e, 0)) } else { None };
        if d.has_volume(e) {
            let pos = chans.iter().filter_map(|&ch| d.norm(e, ch)).fold(None, |m: Option<f64>, n| Some(m.map_or(n, |m| m.max(n))));
            let Some(pos) = pos else { continue };
            let pct = format!("{:.0} %", pos * 100.0);
            let what = if d.0 { "Playback" } else { "Capture" };
            let (text, help) = match d.db_range(e) {
                Some((min, max)) => {
                    let vals: Vec<c_long> = chans.iter().filter_map(|&ch| d.db(e, ch)).collect();
                    let text = match vals.as_slice() {
                        [a, b, ..] if a != b => format!("{} / {} · {pct}", db_text(*a).trim_end_matches(" dB"), db_text(*b)),
                        [a, ..] => format!("{} · {pct}", db_text(*a)),
                        [] => pct.clone(),
                    };
                    let steps = d.raw_range(e).map(|(a, b)| format!(", {} steps", b - a + 1)).unwrap_or_default();
                    (text, format!("{what}: {} … {}{steps}", db_text(min), db_text(max)))
                }
                None => {
                    let raw = chans.first().and_then(|&ch| d.raw(e, ch)).unwrap_or(0);
                    let (a, b) = d.raw_range(e).unwrap_or((0, 0));
                    (format!("{raw} of {b} · {pct}"), format!("{what}: steps {a} … {b} (no dB scale)"))
                }
            };
            let sw = match switch {
                Some(_) if d.0 => "; the switch: on plays, off mutes",
                Some(_) => "; the switch: on records, off mutes",
                None => "",
            };
            let help = format!("{help}{}{sw}. As alsamixer: drag to set; the channels keep their balance (left / right shown when they differ)", if chans.len() > 1 { format!(", {} channels", chans.len()) } else { String::new() });
            out.push(Control { key, label, value: Value::Level { pos: pos as f32, text, switch, help } });
        } else if let Some(on) = switch {
            out.push(Control { key, label, value: Value::Switch(on) });
        }
    }
    out
}

/// The cards' mixers while the tab is shown.
pub struct Mixer {
    cards: Vec<Card>,
}

/// The cards the tab shows: those with audio, snd-aloop's virtual card left out.
fn listed() -> Vec<crate::bridges::Card> {
    crate::bridges::cards().into_iter().filter(|c| !c.is_virtual()).collect()
}

impl Mixer {
    pub fn open() -> Mixer {
        Mixer { cards: listed().into_iter().map(Card::open).collect() }
    }

    pub fn view(&self) -> Vec<CardView> {
        self.cards.iter().map(Card::view).collect()
    }

    /// Read what changed on the cards (a change made anywhere). True if something did.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        let mut lost = false;
        for c in &self.cards {
            if c.mixer.is_null() {
                continue;
            }
            let n = unsafe { snd_mixer_poll_descriptors_count(c.mixer) }.max(0) as usize;
            if n == 0 {
                continue;
            }
            let mut fds = vec![libc::pollfd { fd: -1, events: 0, revents: 0 }; n];
            let got = unsafe { snd_mixer_poll_descriptors(c.mixer, fds.as_mut_ptr(), n as c_uint) }.max(0) as usize;
            fds.truncate(got);
            if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) } <= 0 {
                continue;
            }
            if fds.iter().any(|f| f.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0) {
                lost = true;
                continue;
            }
            if unsafe { snd_mixer_handle_events(c.mixer) } < 0 {
                lost = true;
            }
            any = true;
        }
        if lost {
            self.rescan(true);
            any = true;
        }
        any
    }

    /// Cards plugged in or out: open the new, close the gone. True if the list changed.
    pub fn rescan(&mut self, force: bool) -> bool {
        let now = listed();
        let same = now.len() == self.cards.len() && now.iter().zip(&self.cards).all(|(a, b)| *a == b.card);
        if same && !force {
            return false;
        }
        let mut old: Vec<Card> = std::mem::take(&mut self.cards);
        for c in now {
            match old.iter().position(|o| o.card == c && !force && o.error.is_none()) {
                Some(i) => self.cards.push(old.remove(i)),
                None => self.cards.push(Card::open(c)),
            }
        }
        true
    }

    fn elem(&self, key: &str) -> Option<(*mut SndElem, char)> {
        let mut it = key.splitn(4, ':');
        let card: u32 = it.next()?.parse().ok()?;
        let kind = it.next()?.chars().next()?;
        let index: u32 = it.next()?.parse().ok()?;
        let name = it.next()?;
        let c = self.cards.iter().find(|c| c.card.index == card)?;
        Some((c.find(index, name)?, kind))
    }

    /// A volume to alsamixer's position `pos` (0..1): its loudest channel there, the others in
    /// proportion (their balance kept).
    pub fn set_level(&mut self, key: &str, pos: f64) -> bool {
        let Some((e, kind)) = self.elem(key) else { return false };
        let d = Dir(kind == 'p');
        let chans = d.channels(e);
        let now: Vec<(c_int, f64)> = chans.iter().filter_map(|&ch| Some((ch, d.norm(e, ch)?))).collect();
        let top = now.iter().map(|&(_, n)| n).fold(0.0, f64::max);
        let pos = pos.clamp(0.0, 1.0);
        if (pos - top).abs() < 1e-4 {
            return true;
        }
        let dir = if pos > top { 1 } else { -1 };
        let mut ok = true;
        for (ch, n) in now {
            let want = if top > 1e-3 { n * pos / top } else { pos };
            ok &= d.set_norm(e, ch, want, dir);
        }
        ok
    }

    pub fn set_switch(&mut self, key: &str, on: bool) -> bool {
        let Some((e, kind)) = self.elem(key) else { return false };
        let r = unsafe { if kind == 'p' { snd_mixer_selem_set_playback_switch_all(e, on as c_int) } else { snd_mixer_selem_set_capture_switch_all(e, on as c_int) } };
        r >= 0
    }

    pub fn set_item(&mut self, key: &str, item: u32) -> bool {
        let Some((e, _)) = self.elem(key) else { return false };
        let mut ok = false;
        for ch in 0..CHANNELS {
            let mut cur = 0;
            if unsafe { snd_mixer_selem_get_enum_item(e, ch, &mut cur) } >= 0 {
                ok |= unsafe { snd_mixer_selem_set_enum_item(e, ch, item) } >= 0;
            }
        }
        ok
    }
}

/// JackBus's saved mixer state of card `id` (jackbus-mixer, ~/.config/jackbus/mixer/<id>.state),
/// put back whenever JackBus opens the card: when it was saved, if it was.
pub fn saved_state(id: &str) -> Option<String> {
    let path = crate::compat::config_home()?.join(format!("jackbus/mixer/{id}.state"));
    let text = std::fs::read_to_string(path).ok()?;
    let first = text.lines().next().unwrap_or("");
    Some(first.split(", saved ").nth(1).and_then(|r| r.split('.').next()).unwrap_or("").to_string())
}

/// `jackbus-mixer save|restore <id>`: Ok(its message) or Err(why)
pub fn state_command(what: &str, id: &str) -> Result<String, String> {
    match std::process::Command::new("jackbus-mixer").args([what, id]).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(format!("jackbus-mixer: {e} (JackBus's package has it)")),
    }
}
