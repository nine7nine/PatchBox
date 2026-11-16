//! JackBus's shims: applications that speak PipeWire (jackbus-pw), PulseAudio (jackbus-pa) or
//! ALSA (the "jackbus" PCM) going straight to JackBus. Their settings file
//! (~/.config/jackbus/compat.conf): each shim reads its own key first ("pw.", "pa." or "alsa."
//! then the key), then the shared one; the settings page writes a shim's own. Two per-user
//! switches beside it, ALSA's default device and PulseAudio applications ("pulse"): on JackBus
//! while it runs, on PipeWire while it doesn't.

use std::io;
use std::path::{Path, PathBuf};

fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")),
    }
}

/// The settings file the shims read first.
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

/// A key's value in one file's text (its last line), trimmed; None when not set there.
fn file_str(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key && !v.trim().is_empty() {
                found = Some(v.trim().to_string());
            }
        }
    }
    found
}

/// A shim's setting as it reads it: the user's file, else the system's; its own key
/// ("<shim>.<key>") over the shared one. `shim` "": a key of no shim's.
pub fn get_str(shim: &str, key: &str) -> Option<String> {
    let own = format!("{shim}.{key}");
    let user = conf_path().and_then(|p| std::fs::read_to_string(p).ok());
    let files: Vec<String> = user.into_iter().chain(std::fs::read_to_string(SYSTEM_CONF).ok()).collect();
    files.iter().find_map(|t| if shim.is_empty() { None } else { file_str(t, &own) }.or_else(|| file_str(t, key)))
}

pub fn get_bool(shim: &str, key: &str, def: bool) -> bool {
    get_str(shim, key).and_then(|v| parse_bool(&v)).unwrap_or(def)
}

/// Set a key in the user's file (None: remove it), keeping its other lines.
pub fn set_str(key: &str, value: Option<&str>) -> io::Result<()> {
    let path = conf_path().ok_or_else(|| io::Error::other("no home directory"))?;
    let old = std::fs::read_to_string(&path).unwrap_or_else(|_| "# JackBus's shims (jackbus-pw, jackbus-pa, jackbus-alsa).\n".to_string());
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for l in old.lines() {
        let bare = l.split('#').next().unwrap_or("");
        if bare.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if let (Some(v), false) = (value, done) {
                out.push(format!("{key} = {v}"));
                done = true;
            }
        } else {
            out.push(l.to_string());
        }
    }
    if let (Some(v), false) = (value, done) {
        out.push(format!("{key} = {v}"));
    }
    write_file(&path, &(out.join("\n") + "\n"))
}

pub fn set(key: &str, value: bool) -> io::Result<()> {
    set_str(key, Some(if value { "yes" } else { "no" }))
}

fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Which shims are installed (the jackbus package's paths).
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
// ($XDG_CONFIG_HOME/alsa/asoundrc, read after the system's: it wins over PipeWire's default).
// While JackBus isn't running the device falls back to PipeWire's, the card's, or none
// (alsa.fallback in the settings file remembers the choice).
const ALSA_BEGIN: &str = "# PatchBox: ALSA's default device is JackBus (jackbus-alsa) - begin";
const ALSA_END: &str = "# PatchBox: ALSA's default device is JackBus - end";

pub fn alsa_path() -> Option<PathBuf> {
    Some(config_home()?.join("alsa/asoundrc"))
}

pub fn alsa_default() -> bool {
    alsa_path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t.contains(ALSA_BEGIN))
}

/// "pipewire", "sysdefault" (the first card) or "none"; unset: PipeWire's when pipewire-alsa is
/// installed, else the card's.
pub fn alsa_fallback() -> String {
    let pw = ["/etc/alsa/conf.d/50-pipewire.conf", "/usr/share/alsa/alsa.conf.d/50-pipewire.conf"];
    get_str("alsa", "fallback").unwrap_or_else(|| if pw.iter().any(|p| Path::new(p).exists()) { "pipewire" } else { "sysdefault" }.to_string())
}

pub fn set_alsa_fallback(v: &str) -> io::Result<()> {
    set_str("alsa.fallback", Some(v))?;
    if alsa_default() {
        set_alsa_default(true)?;
    }
    Ok(())
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
        let fb = if fallback.is_empty() || fallback == "none" { String::new() } else { format!("\tfallback \"{fallback}\"\n") };
        kept.push_str(&format!("{ALSA_BEGIN}\npcm.!default {{\n\ttype jackbus\n{fb}}}\n{ALSA_END}\n"));
    }
    kept
}

pub fn set_alsa_default(on: bool) -> io::Result<()> {
    let path = alsa_path().ok_or_else(|| io::Error::other("no home directory"))?;
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let text = alsa_text(&old, on, &alsa_fallback());
    if text.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    write_file(&path, &text)
}

// PulseAudio applications: the jackbus package puts jackbus-pa's libpulse on the dynamic
// linker's path (/etc/ld.so.conf.d), so every program loading libpulse gets it. It reads
// "pulse" as each program starts: off (the default), PulseAudio's own libpulse as before; on,
// JackBus while it runs and PipeWire while it doesn't. The first version used an
// environment.d file instead (a library path, which some launchers replace): it is removed.

pub fn pulse_env_path() -> Option<PathBuf> {
    Some(config_home()?.join("environment.d/60-jackbus-pa.conf"))
}

pub fn pulse_on() -> bool {
    get_bool("", "pulse", false)
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
        let t = "# c\nresample = no # keep\nautoconnect=yes\npa.output = card\n";
        assert_eq!(file_str(t, "resample").as_deref(), Some("no"));
        assert_eq!(file_str(t, "pa.output").as_deref(), Some("card"));
        assert_eq!(file_str(t, "other"), None);
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
        assert!(alsa_text(mine, true, "none").contains("pcm.!default {\n\ttype jackbus\n}"));
    }
}
