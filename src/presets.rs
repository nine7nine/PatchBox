//! JACK server presets (qjackctl's): named jackdbus configurations, the driver and every
//! parameter set away from its default, the same as a session remembers (pbx_session::jackcfg).
//! Kept in ~/.config/patchbox/jack-presets.json; applying one stops the server, sets them
//! (others back to their defaults) and starts it, unless nothing differs.

use pbx_session::jackcfg::JackSettings;
use std::collections::BTreeMap;

pub type Presets = BTreeMap<String, JackSettings>;

fn path() -> Option<std::path::PathBuf> {
    Some(crate::layout::config_dir()?.join("jack-presets.json"))
}

pub fn load() -> Presets {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(p: &Presets) -> Result<(), String> {
    let path = path().ok_or("no configuration directory")?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

/// The preset the server's settings are, if any.
pub fn matching<'a>(p: &'a Presets, now: &JackSettings) -> Option<&'a str> {
    p.iter().find(|(_, s)| s.driver == now.driver && s.params == now.params).map(|(n, _)| n.as_str())
}

/// "alsa · hw:PCH · 48000 Hz · 256 frames": what tells presets apart.
pub fn summary(s: &JackSettings) -> String {
    let get = |k: &str| s.params.get(k).and_then(|v| v.split_once(':')).map(|(_, v)| v.to_string());
    let mut parts = vec![if s.driver.is_empty() { "default driver".to_string() } else { s.driver.clone() }];
    if let Some(d) = get("driver/device") {
        parts.push(d);
    }
    parts.push(get("driver/rate").map(|r| format!("{r} Hz")).unwrap_or_else(|| "default rate".into()));
    parts.push(get("driver/period").map(|p| format!("{p} frames")).unwrap_or_else(|| "default period".into()));
    let rest = s.params.len() - ["driver/device", "driver/rate", "driver/period"].iter().filter(|k| s.params.contains_key(**k)).count();
    if rest > 0 {
        parts.push(format!("{rest} more"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(driver: &str, kv: &[(&str, &str)]) -> JackSettings {
        JackSettings { remember: true, driver: driver.into(), params: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn summary_and_match() {
        let studio = set("alsa", &[("driver/device", "s:hw:PCH"), ("driver/rate", "u:48000"), ("driver/period", "u:64"), ("engine/realtime", "b:true")]);
        assert_eq!(summary(&studio), "alsa · hw:PCH · 48000 Hz · 64 frames · 1 more");
        let mut p = Presets::new();
        p.insert("Studio".into(), studio.clone());
        p.insert("Desktop".into(), set("alsa", &[("driver/period", "u:1024")]));
        assert_eq!(matching(&p, &studio), Some("Studio"));
        assert_eq!(matching(&p, &set("dummy", &[])), None);
    }
}
