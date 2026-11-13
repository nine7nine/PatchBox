//! Fake JACK clients for testing PatchBox against a PRIVATE test server (tests/nested/run.sh).
//! Never point this at a live session: it adds clients and makes connections.
//!
//!   pbx-fake-clients [demo]       the demo scenario below; runs until killed
//!
//! The clients register no process callback, so none of them runs in the RT graph.

use pbx_jack::{Client, keys};
use std::sync::Arc;

struct Spec {
    name: &'static str,
    pretty: Option<&'static str>,
    /// (port, audio, input)
    ports: &'static [(&'static str, bool, bool)],
}

const DEMO: &[Spec] = &[
    Spec { name: "PulseAudio JACK Source", pretty: None, ports: &[("front-left", true, true), ("front-right", true, true)] },
    Spec { name: "PulseAudio JACK Sink", pretty: None, ports: &[("front-left", true, false), ("front-right", true, false)] },
    Spec {
        name: "Carla",
        pretty: Some("Carla: 3 Band Splitter"),
        ports: &[
            ("input_1", true, true), ("input_2", true, true),
            ("output_1", true, false), ("output_2", true, false), ("output_3", true, false),
            ("output_4", true, false), ("output_5", true, false), ("output_6", true, false),
            ("events-in", false, true),
        ],
    },
    Spec {
        name: "Dragonfly Room Reverb",
        pretty: None,
        ports: &[("Input L", true, true), ("Input R", true, true), ("Output L", true, false), ("Output R", true, false), ("events-in", false, true)],
    },
    Spec { name: "Hydrogen", pretty: None, ports: &[("Hydrogen Midi-In", false, true), ("out_L", true, false), ("out_R", true, false)] },
    Spec {
        name: "ardour",
        pretty: None,
        ports: &[
            ("Master/audio_in 1", true, true), ("Master/audio_in 2", true, true),
            ("Master/audio_out 1", true, false), ("Master/audio_out 2", true, false),
            ("Kick/audio_in 1", true, true), ("Snare/audio_in 1", true, true), ("HiHat/audio_in 1", true, true),
            ("Guitar/audio_in 1", true, true), ("voice/audio_in 1", true, true),
            ("MIDI Clock in", false, true), ("MTC in", false, true), ("LTC in", true, true),
        ],
    },
    Spec { name: "a2j", pretty: None, ports: &[("Midi Through [14] (capture): Midi Through Port-0", false, false), ("USB Keystation 61es [20] (capture): USB Keystation 61es MIDI 1", false, false)] },
];

const CONNS: &[(&str, &str)] = &[
    ("system:capture_1", "Carla:input_1"),
    ("system:capture_2", "Carla:input_2"),
    ("Carla:output_1", "Dragonfly Room Reverb:Input L"),
    ("Carla:output_2", "Dragonfly Room Reverb:Input R"),
    ("Dragonfly Room Reverb:Output L", "system:playback_1"),
    ("Dragonfly Room Reverb:Output R", "system:playback_2"),
    ("Hydrogen:out_L", "ardour:Kick/audio_in 1"),
    ("Hydrogen:out_R", "ardour:Snare/audio_in 1"),
    ("ardour:Master/audio_out 1", "system:playback_1"),
    ("ardour:Master/audio_out 2", "system:playback_2"),
    ("PulseAudio JACK Sink:front-left", "system:playback_1"),
    ("PulseAudio JACK Sink:front-right", "system:playback_2"),
    ("system:capture_1", "PulseAudio JACK Source:front-left"),
    ("system:capture_2", "PulseAudio JACK Source:front-right"),
    ("a2j:USB Keystation 61es [20] (capture): USB Keystation 61es MIDI 1", "Hydrogen:Hydrogen Midi-In"),
];

fn main() {
    let waker: pbx_jack::Waker = Arc::new(|| {});
    let mut clients = Vec::new();
    for s in DEMO {
        let c = match Client::open(s.name, waker.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("pbx-fake-clients: {}: {e}", s.name);
                std::process::exit(1);
            }
        };
        for &(p, audio, input) in s.ports {
            if !c.register_port(p, audio, input) {
                eprintln!("pbx-fake-clients: {}:{p}: register failed", s.name);
            }
        }
        if let Some(pretty) = s.pretty {
            if let Some(uuid) = c.client_uuid(c.name()) {
                c.set_property(uuid, keys::PRETTY_NAME, pretty);
            }
        }
        clients.push(c);
    }
    let any = &clients[0];
    for &(a, b) in CONNS {
        if !any.connect(a, b) {
            eprintln!("pbx-fake-clients: connect {a} -> {b} failed");
        }
    }
    eprintln!("pbx-fake-clients: ready ({} clients)", clients.len());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
