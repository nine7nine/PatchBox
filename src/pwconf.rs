//! PipeWire's configuration files, for the settings page: where the JACK bridge is configured,
//! a drop-in of ours that keeps the clock defaults across restarts, and one that turns the JACK
//! bridge off (PipeWire leaves audio to JackBus).
//!
//! PipeWire reads `pipewire.conf` from the first of ~/.config/pipewire, /etc/pipewire and
//! /usr/share/pipewire that has one, then the `pipewire.conf.d/*.conf` drop-ins of all three
//! (PIPEWIRE_CONFIG_DIR replaces the user's directory).

use std::collections::BTreeMap;
use std::path::PathBuf;

fn user_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("PIPEWIRE_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("pipewire"))
}

fn dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = user_dir().into_iter().collect();
    v.push("/etc/pipewire".into());
    v.push("/usr/share/pipewire".into());
    v
}

/// Does `text` load the module (an uncommented mention of its library)?
fn loads(text: &str, module: &str) -> bool {
    let lib = format!("libpipewire-module-{module}");
    text.lines().any(|l| {
        let l = l.trim_start();
        !l.starts_with('#') && l.contains(&lib)
    })
}

/// The file that loads the JACK bridge module (`jackdbus-detect` or `jack-tunnel`): a drop-in
/// if one does, else the pipewire.conf in use.
pub fn bridge_conf(module: &str) -> Option<PathBuf> {
    for d in dirs() {
        let Ok(rd) = std::fs::read_dir(d.join("pipewire.conf.d")) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "conf")).collect();
        files.sort();
        if let Some(p) = files.into_iter().rev().find(|p| std::fs::read_to_string(p).is_ok_and(|t| loads(&t, module))) {
            return Some(p);
        }
    }
    let main = dirs().into_iter().map(|d| d.join("pipewire.conf")).find(|p| p.is_file())?;
    std::fs::read_to_string(&main).is_ok_and(|t| loads(&t, module)).then_some(main)
}

/// Our drop-in that leaves audio to JackBus: PipeWire's JACK bridge, module-jackdbus-detect, is
/// loaded only while `module.jackdbus-detect` is true (as PipeWire's own pipewire.conf has it);
/// this sets it false. While JackBus runs, PipeWire then has no audio work (JackBus has the
/// cards; the shims take PulseAudio and ALSA applications); while it doesn't, PipeWire plays on
/// the cards as before. Applies when PipeWire restarts.
pub fn no_bridge_dropin() -> Option<PathBuf> {
    Some(user_dir()?.join("pipewire.conf.d").join("60-patchbox-no-jack-bridge.conf"))
}

pub fn no_bridge() -> bool {
    no_bridge_dropin().is_some_and(|p| p.exists())
}

pub fn set_no_bridge(on: bool) -> Result<(), String> {
    let p = no_bridge_dropin().ok_or("no configuration directory")?;
    if !on {
        return match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let body = "# Written by PatchBox (settings page, Shims tab): PipeWire leaves audio to JackBus.\n\
                # Its JACK bridge (module-jackdbus-detect) is not loaded: while JackBus runs, PipeWire has\n\
                # no audio work; while it doesn't, PipeWire plays on the sound cards as before.\n\
                context.properties = {\n    module.jackdbus-detect = false\n}\n";
    let dir = p.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("conf.tmp");
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| e.to_string())
}

/// Our drop-in for the clock defaults.
pub fn clock_dropin() -> Option<PathBuf> {
    Some(user_dir()?.join("pipewire.conf.d").join("60-patchbox-clock.conf"))
}

/// Write the clock defaults now in force (the `settings` metadata) to our drop-in.
pub fn write_clock_dropin(settings: &BTreeMap<String, String>) -> Result<PathBuf, String> {
    let p = clock_dropin().ok_or("no configuration directory")?;
    let mut body = String::from("# Written by PatchBox (settings page, PipeWire tab): the clock defaults.\ncontext.properties = {\n");
    for key in ["clock.rate", "clock.allowed-rates", "clock.quantum", "clock.min-quantum", "clock.max-quantum"] {
        if let Some(v) = settings.get(key).filter(|v| !v.trim().is_empty()) {
            body.push_str(&format!("    default.{key} = {}\n", v.trim()));
        }
    }
    body.push_str("}\n");
    let dir = p.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("conf.tmp");
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| e.to_string())?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commented_modules_dont_count() {
        assert!(loads("{ name = libpipewire-module-jackdbus-detect\n", "jackdbus-detect"));
        assert!(!loads("#{ name = libpipewire-module-jackdbus-detect\n", "jackdbus-detect"));
        assert!(!loads("  # libpipewire-module-jack-tunnel\n", "jack-tunnel"));
    }
}
