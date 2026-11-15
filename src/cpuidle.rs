//! The CPUs' idle states (cpuidle, in sysfs), as levels for JackBus's CPU wake-up latency limit
//! (the engine's cpu-dma-latency, a PM QoS request). A limit of L µs keeps every CPU out of the
//! idle states that take longer than L to wake from, so the levels that mean something are the
//! states' own latencies: they are read from the machine PatchBox runs on (intel_idle,
//! acpi_idle, AMD's), never assumed.

/// An idle state: its name and the longest wake-up latency any CPU gives for it (µs).
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub latency: u32,
}

/// A choice in the menu: the limit asked for (µs; -1: none) and what it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub usec: i32,
    pub label: String,
}

/// The idle states of this machine's CPUs, shallowest first (POLL left out).
pub fn states() -> Vec<State> {
    let mut found = Vec::new();
    let Ok(cpus) = std::fs::read_dir("/sys/devices/system/cpu") else { return Vec::new() };
    for cpu in cpus.flatten() {
        let name = cpu.file_name().to_string_lossy().into_owned();
        if !name.strip_prefix("cpu").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let Ok(dir) = std::fs::read_dir(cpu.path().join("cpuidle")) else { continue };
        for st in dir.flatten() {
            let p = st.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).ok().map(|t| t.trim().to_string());
            let (Some(name), Some(lat)) = (read("name"), read("latency").and_then(|l| l.parse().ok())) else { continue };
            found.push((name, lat, read("disable").as_deref() == Some("1")));
        }
    }
    collate(found)
}

/// One state per name from every CPU's (name, latency, disabled): its latency the largest any
/// CPU gives (hybrid CPUs' cores differ, and the limit must allow it on all of them); states
/// disabled on every CPU and POLL left out; shallowest first.
pub fn collate(found: Vec<(String, u32, bool)>) -> Vec<State> {
    let mut by: Vec<(String, u32, bool)> = Vec::new(); // name, latency, enabled somewhere
    for (name, lat, disabled) in found {
        if name == "POLL" {
            continue;
        }
        match by.iter_mut().find(|e| e.0 == name) {
            Some(e) => {
                e.1 = e.1.max(lat);
                e.2 |= !disabled;
            }
            None => by.push((name, lat, !disabled)),
        }
    }
    let mut v: Vec<State> = by.into_iter().filter(|e| e.2).map(|(name, latency, _)| State { name, latency }).collect();
    v.sort_by_key(|s| s.latency);
    v
}

/// The states as first read (for tooltips, read every frame they show); the menu reads afresh.
pub fn cached() -> &'static [State] {
    static STATES: std::sync::OnceLock<Vec<State>> = std::sync::OnceLock::new();
    STATES.get_or_init(states)
}

/// A state's name as shown: intel_idle's names from ACPI tables lose their "_ACPI".
pub fn short(name: &str) -> &str {
    name.strip_suffix("_ACPI").unwrap_or(name)
}

/// The levels, from performance to power saving: no idle state at all (0 µs: the CPUs poll),
/// then up to each state in turn (the limit: that state's latency), then no limit. The deepest
/// state has no level of its own: no limit allows it.
pub fn levels(states: &[State]) -> Vec<Level> {
    let mut v = vec![Level { usec: 0, label: "0 µs · performance: no idle states, the CPUs poll (most power and heat)".into() }];
    for (i, s) in states.iter().enumerate().take(states.len().saturating_sub(1)) {
        let usec = i32::try_from(s.latency).unwrap_or(i32::MAX);
        if v.iter().any(|l| l.usec == usec) {
            continue;
        }
        let lighter = if i == 0 { "lowest latency that lets CPUs idle" } else { "balanced" };
        v.push(Level { usec, label: format!("{} µs · idle up to {} ({lighter})", s.latency, short(&s.name)) });
    }
    let deepest = states.last().map(|s| format!(", up to {}", short(&s.name))).unwrap_or_default();
    v.push(Level { usec: -1, label: format!("off · power saving: every idle state{deepest}") });
    v
}

/// What a limit allows, for the toolbar: "POLL", "≤ C1", "any" (or the limit in µs where the
/// machine shows no idle states).
pub fn allows(states: &[State], usec: i32) -> String {
    match usec {
        n if n < 0 => "any".into(),
        0 => "POLL".into(),
        n => match states.iter().rev().find(|s| i64::from(s.latency) <= i64::from(n)) {
            Some(s) if states.last() == Some(s) => "any".into(),
            Some(s) => format!("≤ {}", short(&s.name)),
            None if states.is_empty() => format!("{n} µs"),
            None => "POLL".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(v: &[(&str, u32)]) -> Vec<State> {
        v.iter().map(|(n, l)| State { name: n.to_string(), latency: *l }).collect()
    }

    #[test]
    fn collates_cpus() {
        // a hybrid CPU: the E-cores' C6 is slower to wake; C8 disabled everywhere
        let found = vec![
            ("POLL".to_string(), 0, false),
            ("C1".to_string(), 1, false),
            ("C6".to_string(), 170, false),
            ("C8".to_string(), 200, true),
            ("POLL".to_string(), 0, false),
            ("C1".to_string(), 1, false),
            ("C6".to_string(), 220, false),
            ("C8".to_string(), 200, true),
        ];
        assert_eq!(collate(found), st(&[("C1", 1), ("C6", 220)]));
    }

    #[test]
    fn levels_follow_the_states() {
        let s = st(&[("C1_ACPI", 1), ("C2_ACPI", 253), ("C3_ACPI", 1048)]);
        let l = levels(&s);
        assert_eq!(l.iter().map(|l| l.usec).collect::<Vec<_>>(), [0, 1, 253, -1]);
        assert_eq!(l[1].label, "1 µs · idle up to C1 (lowest latency that lets CPUs idle)");
        assert_eq!(l[2].label, "253 µs · idle up to C2 (balanced)");
        assert!(l[3].label.contains("up to C3"));
        // no cpuidle (a VM, idle=poll): only the two ends
        assert_eq!(levels(&[]).iter().map(|l| l.usec).collect::<Vec<_>>(), [0, -1]);
    }

    #[test]
    fn what_a_limit_allows() {
        let s = st(&[("C1", 1), ("C1E", 10), ("C6", 133), ("C10", 890)]);
        assert_eq!(allows(&s, -1), "any");
        assert_eq!(allows(&s, 0), "POLL");
        assert_eq!(allows(&s, 1), "≤ C1");
        assert_eq!(allows(&s, 50), "≤ C1E");
        assert_eq!(allows(&s, 2000), "any");
        assert_eq!(allows(&[], 20), "20 µs");
    }
}
