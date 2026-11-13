//! Command-line control of patchboxd.
//!
//!   patchbox-ctl state | sessions | new NAME | open NAME | save | close | abort
//!                duplicate NAME | rename NAME | add [--plain] EXE [ARGS…]
//!                start ID | stop ID | kill ID | save-client ID | show-gui ID | hide-gui ID
//!                trash ID | restore ID | notes TEXT | connect SRC DST | disconnect SRC DST
//!                graph | quit | watch

use pbx_session::proto::{Conn, FromDaemon, Push, Reply, Request};

fn usage() -> ! {
    eprintln!("{}", include_str!("patchbox-ctl.rs").lines().skip(2).take(5).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
    std::process::exit(2)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| a.get(i).cloned().unwrap_or_else(|| usage());
    let req = match a.first().map(String::as_str) {
        Some("state") => Request::State,
        Some("sessions") => Request::ListSessions,
        Some("new") => Request::New { name: arg(1) },
        Some("open") => Request::Open { name: arg(1) },
        Some("save") => Request::Save,
        Some("close") => Request::Close,
        Some("abort") => Request::Abort,
        Some("duplicate") => Request::Duplicate { name: arg(1) },
        Some("rename") => Request::Rename { name: arg(1) },
        Some("notes") => Request::SetNotes { notes: a[1..].join(" ") },
        Some("add") => {
            let plain = a.get(1).is_some_and(|x| x == "--plain");
            let at = if plain { 2 } else { 1 };
            Request::Add { exe: arg(at), args: a[at + 1..].to_vec(), nsm: !plain, label: String::new() }
        }
        Some("start") => Request::Start { id: arg(1) },
        Some("stop") => Request::Stop { id: arg(1) },
        Some("kill") => Request::Kill { id: arg(1) },
        Some("save-client") => Request::SaveClient { id: arg(1) },
        Some("show-gui") => Request::ShowGui { id: arg(1) },
        Some("hide-gui") => Request::HideGui { id: arg(1) },
        Some("trash") => Request::Trash { id: arg(1) },
        Some("restore") => Request::Restore { id: arg(1) },
        Some("connect") => Request::Connect { src: arg(1), dst: arg(2) },
        Some("disconnect") => Request::Disconnect { src: arg(1), dst: arg(2) },
        Some("graph") => Request::Graph,
        Some("quit") => Request::Quit,
        Some("watch") => Request::Subscribe,
        _ => usage(),
    };
    let mut c = match Conn::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("patchbox-ctl: patchboxd is not running ({e})");
            std::process::exit(1);
        }
    };
    let watch = matches!(req, Request::Subscribe);
    match c.call(&req) {
        Ok(Reply::Ok { msg }) => println!("{msg}"),
        Ok(Reply::Error { msg }) => {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
        Ok(Reply::Sessions { root, sessions }) => {
            println!("# {root}");
            for s in sessions {
                println!("{}\t{} clients\t{}", s.name, s.clients, s.notes);
            }
        }
        Ok(Reply::State(st)) => println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default()),
        Err(e) => {
            eprintln!("patchbox-ctl: {e}");
            std::process::exit(1);
        }
    }
    while watch {
        match c.recv() {
            Ok(FromDaemon::Push(Push::Log { text, .. })) => println!("{text}"),
            Ok(FromDaemon::Push(Push::State(st))) => {
                let cl: Vec<String> = st.clients.iter().map(|c| format!("{}={:?}", c.id, c.status)).collect();
                println!("[state] session={:?} busy={:?} {}", st.session, st.busy, cl.join(" "));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}
