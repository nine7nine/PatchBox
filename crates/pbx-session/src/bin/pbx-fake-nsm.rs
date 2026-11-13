//! A fake NSM client for tests (run by patchboxd inside the private test namespace).
//! Announces with :dirty:optional-gui:, creates its project folder on open, writes a file on
//! save, registers JACK ports `in_L`, `in_R`, `out_L`, `out_R` under the client name it is
//! given, and auto-connects out_L → system:playback_1 like some applications do.

use pbx_session::osc::{self, Arg, Msg};
use std::net::UdpSocket;

fn main() {
    let url = std::env::var("NSM_URL").unwrap_or_else(|_| {
        eprintln!("pbx-fake-nsm: no NSM_URL");
        std::process::exit(1)
    });
    let hostport = url.trim_start_matches("osc.udp://").trim_end_matches('/').to_string();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let send = |m: Msg| {
        let _ = sock.send_to(&osc::encode(&m), &hostport);
    };
    let name = std::env::args().nth(1).unwrap_or_else(|| "FakeApp".into());
    send(Msg::new(
        "/nsm/server/announce",
        vec![
            Arg::S(name.clone()),
            Arg::S(":dirty:optional-gui:switch:".into()),
            Arg::S("pbx-fake-nsm".into()),
            Arg::I(1),
            Arg::I(2),
            Arg::I(std::process::id() as i32),
        ],
    ));
    let mut path = String::new();
    let mut jack: Option<pbx_jack::Client> = None;
    let mut buf = vec![0u8; 65536];
    loop {
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        for m in osc::decode(&buf[..n]) {
            eprintln!("pbx-fake-nsm: {} {:?}", m.path, m.args);
            match m.path.as_str() {
                "/nsm/client/open" => {
                    path = m.s(0).unwrap_or("").to_string();
                    let jname = m.s(2).unwrap_or("fake").to_string();
                    let _ = std::fs::create_dir_all(&path);
                    // A switch: same process, new files; the JACK client stays.
                    if jack.is_some() {
                        send(Msg::new("/reply", vec![Arg::S("/nsm/client/open".into()), Arg::S("switched".into())]));
                        continue;
                    }
                    let w: pbx_jack::Waker = std::sync::Arc::new(|| {});
                    if let Ok(c) = pbx_jack::Client::open(&jname, w) {
                        for (p, input) in [("in_L", true), ("in_R", true), ("out_L", false), ("out_R", false)] {
                            c.register_port(p, true, input);
                        }
                        let me = c.name().to_string();
                        c.connect(&format!("{me}:out_L"), "system:playback_1");
                        jack = Some(c);
                    }
                    send(Msg::new("/reply", vec![Arg::S("/nsm/client/open".into()), Arg::S("opened".into())]));
                    send(Msg::new("/nsm/client/is_dirty", vec![]));
                }
                "/nsm/client/save" => {
                    let _ = std::fs::write(format!("{path}/state.txt"), "saved\n");
                    send(Msg::new("/reply", vec![Arg::S("/nsm/client/save".into()), Arg::S("saved".into())]));
                }
                "/nsm/client/show_optional_gui" => send(Msg::new("/nsm/client/gui_is_shown", vec![])),
                "/nsm/client/hide_optional_gui" => send(Msg::new("/nsm/client/gui_is_hidden", vec![])),
                _ => {}
            }
        }
        let _ = &jack;
    }
}
