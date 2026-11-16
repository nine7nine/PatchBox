//! Sound card bridges (JackBus): a card on its own clock, joined to the JACK graph with
//! adaptive resampling. jackdbus keeps the list in its engine parameter "bridges" (specs as
//! `device[/period[/nperiods]][@wake]`, separated by spaces) and reports each bridge's state
//! (GetBridges); the settings page's Devices tab shows every sound card with a switch.

use crate::server::Bridge;

/// A sound card, as /proc/asound/cards lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub index: u32,
    /// ALSA's id ("U44", "PCH"): what bridges name the card by.
    pub id: String,
    pub name: String,
    /// The kernel driver ("USB-Audio", "HDA-Intel", "Loopback": snd-aloop's virtual card).
    pub driver: String,
}

impl Card {
    /// A card with no hardware behind it (snd-aloop's).
    pub fn is_virtual(&self) -> bool {
        self.driver == "Loopback"
    }
}

/// The sound cards there now that have audio: a card with MIDI only (a USB keyboard or
/// controller) has no PCM device, nothing to bridge; its MIDI ports come from a MIDI slave
/// driver.
pub fn cards() -> Vec<Card> {
    std::fs::read_to_string("/proc/asound/cards")
        .map(|t| parse_cards(&t))
        .unwrap_or_default()
        .into_iter()
        .filter(|c| card_has_pcm(c.index))
        .collect()
}

/// Whether /proc/asound/cardN lists a PCM device (pcm0p, pcm3c, ...); a card it can't read
/// is kept.
fn card_has_pcm(index: u32) -> bool {
    match std::fs::read_dir(format!("/proc/asound/card{index}")) {
        Ok(dir) => has_pcm_entry(dir.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned())),
        Err(_) => true,
    }
}

fn has_pcm_entry(names: impl Iterator<Item = String>) -> bool {
    names.into_iter().any(|n| {
        n.strip_prefix("pcm")
            .and_then(|r| r.strip_suffix('p').or_else(|| r.strip_suffix('c')))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// " 1 [U44            ]: USB-Audio - U-44" (and a second line, the long name)
fn parse_cards(text: &str) -> Vec<Card> {
    text.lines()
        .filter_map(|l| {
            let (index, rest) = l.trim_start().split_once(' ')?;
            let index = index.parse().ok()?;
            let rest = rest.trim_start().strip_prefix('[')?;
            let (id, rest) = rest.split_once(']')?;
            let (driver, name) = rest.trim_start_matches(':').split_once(" - ").unwrap_or((rest, ""));
            Some(Card { index, id: id.trim().to_string(), name: name.trim().to_string(), driver: driver.trim().to_string() })
        })
        .collect()
}

/// A bridge spec's parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub device: String,
    pub period: Option<u32>,
    pub nperiods: Option<u32>,
    pub wake: Option<String>,
}

pub const DEFAULT_PERIOD: u32 = 128;
pub const DEFAULT_NPERIODS: u32 = 2;

impl Spec {
    pub fn parse(s: &str) -> Spec {
        let (rest, wake) = match s.split_once('@') {
            Some((r, w)) => (r, Some(w.to_string())),
            None => (s, None),
        };
        let mut parts = rest.split('/');
        let device = parts.next().unwrap_or("").to_string();
        let period = parts.next().and_then(|p| p.parse().ok());
        let nperiods = parts.next().and_then(|p| p.parse().ok());
        Spec { device, period, nperiods, wake }
    }

    /// Written back the short way: what is at its default left out.
    pub fn to_spec(&self) -> String {
        let mut s = self.device.clone();
        let period = self.period.filter(|p| *p != DEFAULT_PERIOD);
        let nperiods = self.nperiods.filter(|n| *n != DEFAULT_NPERIODS);
        if period.is_some() || nperiods.is_some() {
            s += &format!("/{}", self.period.unwrap_or(DEFAULT_PERIOD));
        }
        if let Some(n) = nperiods {
            s += &format!("/{n}");
        }
        if let Some(w) = self.wake.as_deref().filter(|w| *w != "auto") {
            s += &format!("@{w}");
        }
        s
    }

    /// The card it names (its id or index, as written) and the PCM device number: "U44" and
    /// "hw:U44,0" are ("U44", 0), "hw:PCH,3" is ("PCH", 3), "plughw:CARD=PCH,DEV=3" too. None
    /// for names other than a card's ("default").
    pub fn card(&self) -> Option<(String, u32)> {
        let d = &self.device;
        let rest = if let Some(r) = d.strip_prefix("hw:").or_else(|| d.strip_prefix("plughw:")) {
            r
        } else if d.contains(':') {
            return None;
        } else {
            d.as_str()
        };
        let mut it = rest.split(',');
        let first = it.next()?;
        let card = first.strip_prefix("CARD=").unwrap_or(first);
        let dev = it.next().map(|x| x.strip_prefix("DEV=").unwrap_or(x)).and_then(|x| x.parse().ok()).unwrap_or(0);
        (!card.is_empty()).then(|| (card.to_string(), dev))
    }

    /// Whether it is this card's first device (what the Devices tab's switch for it means).
    pub fn is_card(&self, card: &Card) -> bool {
        matches!(self.card(), Some((c, 0)) if c.eq_ignore_ascii_case(&card.id) || c == card.index.to_string())
    }
}

/// What a spec asks for, briefly: "64 frames × 3 · timer" (defaults left out; "" for none).
pub fn describe(spec: &str) -> String {
    let sp = Spec::parse(spec);
    let mut parts = Vec::new();
    if sp.period.is_some() || sp.nperiods.is_some() {
        parts.push(format!("{} frames × {}", sp.period.unwrap_or(DEFAULT_PERIOD), sp.nperiods.unwrap_or(DEFAULT_NPERIODS)));
    }
    if let Some(w) = sp.wake.filter(|w| w != "auto") {
        parts.push(w);
    }
    parts.join(" · ")
}

/// The settings of cards switched off (card id → spec), so that switching one on again brings
/// them back: PatchBox's own (bridges-off.json in its configuration directory).
pub fn remembered() -> std::collections::BTreeMap<String, String> {
    let Some(p) = crate::layout::config_dir().map(|d| d.join("bridges-off.json")) else { return Default::default() };
    std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Keep (Some) or forget (None) a card's settings.
pub fn remember(card: &str, spec: Option<&str>) {
    let Some(dir) = crate::layout::config_dir() else { return };
    let mut m = remembered();
    match spec {
        Some(s) => {
            m.insert(card.to_string(), s.to_string());
        }
        None => {
            m.remove(card);
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    let (p, tmp) = (dir.join("bridges-off.json"), dir.join("bridges-off.json.tmp"));
    if let Ok(body) = serde_json::to_vec_pretty(&m) {
        let _ = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p));
    }
}

/// The bridges listed in the engine parameter.
pub fn specs(list: &str) -> Vec<String> {
    list.split([' ', '\t', ';']).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// A listed bridge as it is: a sentence for its row.
pub fn state(b: Option<&Bridge>, started: bool, master_rate: u32) -> String {
    let Some(b) = b else {
        return if started { "starting…".into() } else { "bridged when the server starts".into() };
    };
    match b.card.as_str() {
        "stopped" => "bridged when the server starts".into(),
        "absent" => "not plugged in: bridged when it is".into(),
        "gone" => "unplugged: its ports stay, silent, until it is back".into(),
        "failed" => "the card failed: trying again".into(),
        _ => {
            // the direction furthest from locked says where it is
            let order = ["lost", "idle", "learning", "starting", "acquiring", "locked", "none"];
            let rank = |s: &str| order.iter().position(|o| *o == s).unwrap_or(0);
            let st = if rank(&b.capture) <= rank(&b.playback) { &b.capture } else { &b.playback };
            let st = if st == "none" { "locked" } else { st.as_str() };
            let ms = |f: f64| f * 1000.0 / master_rate.max(1) as f64;
            // channels and delays, capture/playback (one of them for a card with one direction)
            let (cin, cout) = (b.capture_channels, b.playback_channels);
            let mut s = match (cin, cout) {
                (0, o) => format!("{st} · {} Hz · {o} out", b.rate),
                (i, 0) => format!("{st} · {} Hz · {i} in", b.rate),
                (i, o) => format!("{st} · {} Hz · {i}/{o} ch", b.rate),
            };
            let din = (cin > 0 && b.capture_latency > 0.0).then(|| format!("{:.1}", ms(b.capture_latency)));
            let dout = (cout > 0 && b.playback_latency > 0.0).then(|| format!("{:.1}", ms(b.playback_latency)));
            match (din, dout) {
                (Some(i), Some(o)) => s += &format!(" · {i}/{o} ms"),
                (Some(d), None) | (None, Some(d)) => s += &format!(" · {d} ms"),
                (None, None) => {}
            }
            if st == "locked" {
                s += &format!(" · {:+.1} ppm", b.drift_ppm);
            }
            if b.xruns > 0 {
                s += &format!(" · {} card xrun{}", b.xruns, if b.xruns == 1 { "" } else { "s" });
            }
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_cards() {
        let t = " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n                      HDA Intel PCH at 0xa1210000 irq 151\n 1 [U44            ]: USB-Audio - U-44\n                      ZOOM Corporation U-44 at usb-0000:00:14.0-2, high speed\n";
        let c = parse_cards(t);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1], Card { index: 1, id: "U44".into(), name: "U-44".into(), driver: "USB-Audio".into() });
        assert_eq!(c[0].name, "HDA Intel PCH");
        assert_eq!(c[0].driver, "HDA-Intel");
        let l = parse_cards(" 2 [Loopback       ]: Loopback - Loopback\n                      Loopback 1\n");
        assert!(l[0].is_virtual() && !c[1].is_virtual());
    }

    #[test]
    fn midi_only_cards() {
        let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter();
        // a USB keyboard: MIDI only, nothing to bridge
        assert!(!has_pcm_entry(names(&["id", "midi0", "usbbus", "usbid", "usbmixer"])));
        assert!(has_pcm_entry(names(&["id", "pcm0c", "pcm0p", "pcm3p"])));
        assert!(has_pcm_entry(names(&["id", "pcm1c"])));
        assert!(!has_pcm_entry(names(&["pcmp", "pcm0x", "codec#0"])));
    }

    #[test]
    fn spec_round_trip() {
        for s in ["U44", "U44/64", "U44/128/3", "U44/256/3@timer", "hw:PCH,3@irq", "Headset@timer"] {
            assert_eq!(Spec::parse(s).to_spec(), s, "{s}");
        }
        // defaults left out
        let mut sp = Spec::parse("U44/64/3@timer");
        sp.period = Some(128);
        sp.nperiods = Some(2);
        sp.wake = Some("auto".into());
        assert_eq!(sp.to_spec(), "U44");
        sp.nperiods = Some(4);
        assert_eq!(sp.to_spec(), "U44/128/4");
    }

    #[test]
    fn spec_cards() {
        let card = Card { index: 1, id: "U44".into(), name: String::new(), driver: String::new() };
        for s in ["U44", "hw:U44", "hw:U44,0", "hw:u44,0/64", "plughw:CARD=U44,DEV=0", "hw:1"] {
            assert!(Spec::parse(s).is_card(&card), "{s}");
        }
        for s in ["hw:U44,1", "hw:PCH", "default", "hw:2"] {
            assert!(!Spec::parse(s).is_card(&card), "{s}");
        }
        assert_eq!(Spec::parse("hw:PCH,3").card(), Some(("PCH".into(), 3)));
        assert_eq!(Spec::parse("plughw:CARD=PCH,DEV=3").card(), Some(("PCH".into(), 3)));
    }

    #[test]
    fn described() {
        assert_eq!(describe("U44"), "");
        assert_eq!(describe("U44/64/3@timer"), "64 frames × 3 · timer");
        assert_eq!(describe("U44@irq"), "irq");
        assert_eq!(describe("hw:PCH,3/256"), "256 frames × 2");
    }

    #[test]
    fn listed() {
        assert_eq!(specs(" U44  Headset/64;hw:PCH,3 "), vec!["U44", "Headset/64", "hw:PCH,3"]);
        assert!(specs("").is_empty());
    }

    #[test]
    fn states() {
        let b = Bridge {
            card: "running".into(),
            capture: "locked".into(),
            playback: "acquiring".into(),
            rate: 48000,
            capture_channels: 4,
            playback_channels: 4,
            capture_latency: 638.0,
            playback_latency: 636.0,
            drift_ppm: 5.9,
            ..Default::default()
        };
        assert_eq!(state(Some(&b), true, 48000), "acquiring · 48000 Hz · 4/4 ch · 13.3/13.2 ms");
        let b = Bridge { playback: "locked".into(), ..b };
        assert_eq!(state(Some(&b), true, 48000), "locked · 48000 Hz · 4/4 ch · 13.3/13.2 ms · +5.9 ppm");
        let mic = Bridge { capture: "locked".into(), playback: "none".into(), playback_channels: 0, ..b };
        assert_eq!(state(Some(&mic), true, 48000), "locked · 48000 Hz · 4 in · 13.3 ms · +5.9 ppm");
        assert_eq!(state(None, false, 48000), "bridged when the server starts");
    }
}
