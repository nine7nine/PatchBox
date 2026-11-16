//! JackBus's shims: applications that speak PipeWire (jackbus-pw), PulseAudio (jackbus-pa) or
//! ALSA (the "jackbus" PCM) going straight to JackBus. Their shared settings
//! (~/.config/jackbus/compat.conf), and two per-user switches, ALSA's default device and
//! PulseAudio applications ("pulse" in the same file): on JackBus while it runs, on PipeWire
//! while it doesn't.

use std::io;
use std::path::{Path, PathBuf};

fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")),
    }
}

/// The settings file the layers read first.
pub fn conf_path() -> Option<PathBuf> {
    Some(config_home()?.join("jackbus/compat.conf"))
}

const SYSTEM_CONF: &str = "/etc/jackbus/compat.conf";

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn file_value(text: &str, key: &str) -> Option<bool> {
    let mut found = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                if let Some(b) = parse_bool(v) {
                    found = Some(b);
                }
            }
        }
    }
    found
}

/// A setting as the layers see it: the user's file, else the system's, else `def`.
pub fn get_or(key: &str, def: bool) -> bool {
    let user = conf_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| file_value(&t, key));
    user.or_else(|| std::fs::read_to_string(SYSTEM_CONF).ok().and_then(|t| file_value(&t, key))).unwrap_or(def)
}

/// resample and autoconnect: on unless set.
pub fn get(key: &str) -> bool {
    get_or(key, true)
}

/// Set a key in the user's file, keeping its other lines.
pub fn set(key: &str, value: bool) -> io::Result<()> {
    let path = conf_path().ok_or_else(|| io::Error::other("no home directory"))?;
    let old = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        "# JackBus's compatibility layers (jackbus-pw, jackbus-pa, jackbus-alsa).\n".to_string()
    });
    let wanted = format!("{key} = {}", if value { "yes" } else { "no" });
    let mut done = false;
    let mut out: Vec<String> = old
        .lines()
        .map(|l| {
            let bare = l.split('#').next().unwrap_or("");
            if bare.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
                done = true;
                wanted.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !done {
        out.push(wanted);
    }
    write_file(&path, &(out.join("\n") + "\n"))
}

fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Which layers are installed (the jackbus package's paths).
pub struct Installed {
    pub pw: bool,
    pub pa: bool,
    pub alsa: bool,
}

pub fn installed() -> Installed {
    let alsa_dirs = ["/usr/lib/alsa-lib", "/usr/lib64/alsa-lib"];
    Installed {
        pw: Path::new("/usr/lib/jackbus-pw/libpipewire-0.3.so.0").exists(),
        pa: Path::new("/usr/lib/jackbus-pa/libpulse.so.0").exists(),
        alsa: alsa_dirs.iter().any(|d| Path::new(d).join("libasound_module_pcm_jackbus.so").exists()),
    }
}

// ALSA's default device: a block of ours in the user's ALSA configuration
// ($XDG_CONFIG_HOME/alsa/asoundrc, read after the system's: it wins over
// PipeWire's default). The device falls back to PipeWire's (or the card's)
// while JackBus isn't running.
const ALSA_BEGIN: &str = "# PatchBox: ALSA's default device is JackBus (jackbus-alsa) - begin";
const ALSA_END: &str = "# PatchBox: ALSA's default device is JackBus - end";

pub fn alsa_path() -> Option<PathBuf> {
    Some(config_home()?.join("alsa/asoundrc"))
}

pub fn alsa_default() -> bool {
    alsa_path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t.contains(ALSA_BEGIN))
}

/// The device ALSA applications use while JackBus isn't running: PipeWire's when pipewire-alsa
/// is installed, else the first card's own.
fn alsa_fallback() -> &'static str {
    let pw = ["/etc/alsa/conf.d/50-pipewire.conf", "/usr/share/alsa/alsa.conf.d/50-pipewire.conf"];
    if pw.iter().any(|p| Path::new(p).exists()) {
        "pipewire"
    } else {
        "sysdefault"
    }
}

/// The user's ALSA configuration with our block removed, then added again when `on`.
fn alsa_text(old: &str, on: bool, fallback: &str) -> String {
    let mut kept = String::new();
    let mut inside = false;
    for line in old.lines() {
        if line == ALSA_BEGIN {
            inside = true;
        } else if line == ALSA_END {
            inside = false;
        } else if !inside {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if on {
        kept.push_str(&format!("{ALSA_BEGIN}\npcm.!default {{\n\ttype jackbus\n\tfallback \"{fallback}\"\n}}\n{ALSA_END}\n"));
    }
    kept
}

pub fn set_alsa_default(on: bool) -> io::Result<()> {
    let path = alsa_path().ok_or_else(|| io::Error::other("no home directory"))?;
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let text = alsa_text(&old, on, alsa_fallback());
    if text.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    write_file(&path, &text)
}

// PulseAudio applications: the jackbus package puts jackbus-pa's libpulse on the dynamic
// linker's path (/etc/ld.so.conf.d), so every program loading libpulse gets it, whatever
// library path its launcher gives it. It reads "pulse" from the settings file as each program
// starts: off (the default), PulseAudio's own libpulse as before; on, JackBus while it runs and
// PipeWire while it doesn't. The first version used an environment.d file instead (a library
// path, which some launchers replace): it is removed.

pub fn pulse_env_path() -> Option<PathBuf> {
    Some(config_home()?.join("environment.d/60-jackbus-pa.conf"))
}

pub fn pulse_on() -> bool {
    get_or("pulse", false)
}

pub fn set_pulse(on: bool) -> io::Result<()> {
    set("pulse", on)?;
    remove_old_pulse_env()
}

fn remove_old_pulse_env() -> io::Result<()> {
    match pulse_env_path().map(std::fs::remove_file) {
        Some(Err(e)) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// From the first version: a switch that was on (its environment.d file) is the setting now.
pub fn migrate() {
    if pulse_env_path().is_some_and(|p| p.exists()) {
        if let Err(e) = set("pulse", true).and_then(|_| remove_old_pulse_env()) {
            eprintln!("[patchbox] PulseAudio applications on JackBus: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        let t = "# c\nresample = no # keep\nautoconnect=yes\n";
        assert_eq!(file_value(t, "resample"), Some(false));
        assert_eq!(file_value(t, "autoconnect"), Some(true));
        assert_eq!(file_value(t, "other"), None);
        assert_eq!(parse_bool(" On "), Some(true));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn alsa_block() {
        let mine = "pcm.mine { type hw card 1 }\n";
        let on = alsa_text(mine, true, "pipewire");
        assert!(on.starts_with(mine));
        assert!(on.contains("pcm.!default {\n\ttype jackbus\n\tfallback \"pipewire\"\n}"));
        assert_eq!(alsa_text(&on, true, "pipewire"), on, "added once");
        assert_eq!(alsa_text(&on, false, "pipewire"), mine, "removed, the user's lines kept");
        assert!(alsa_text(&alsa_text("", true, "sysdefault"), false, "").trim().is_empty());
    }
}
