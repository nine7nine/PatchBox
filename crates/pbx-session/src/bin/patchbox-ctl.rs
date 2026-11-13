//! Command-line control of patchboxd.
//!
//!   patchbox-ctl state | sessions | new NAME [--template T] | open NAME | save | close | abort
//!                templates | save-template NAME
//!                duplicate NAME | rename NAME | add [--plain] EXE [ARGS…]
//!                start ID | stop ID | kill ID | save-client ID | show-gui ID | hide-gui ID
//!                trash ID | restore ID | notes TEXT | connect SRC DST | disconnect SRC DST
//!                graph | snapshot [LABEL] | snapshots | rewind ID | auto-snapshot on|off | jack-memory on|off
//!                rules | rule-add OUT IN | rule-del OUT IN   (names may hold *)
//!                props ID [label=L] [save=SIG] [stop=SIG] [terminal=on|off] [env=K=V…] [-- CMD ARGS…]
//!                quit | watch

use pbx_session::proto::{Conn, FromDaemon, Push, Reply, Request};

fn usage() -> ! {
    eprintln!("{}", include_str!("patchbox-ctl.rs").lines().skip(2).take(9).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
    std::process::exit(2)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| a.get(i).cloned().unwrap_or_else(|| usage());
    let req = match a.first().map(String::as_str) {
        Some("state") => Request::State,
        Some("sessions") => Request::ListSessions,
        Some("new") => Request::New { name: arg(1), template: a.iter().position(|x| x == "--template").and_then(|i| a.get(i + 1).cloned()) },
        Some("templates") => Request::ListTemplates,
        Some("save-template") => Request::SaveTemplate { name: arg(1) },
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
        Some("snapshot") => Request::Snapshot { label: a[1..].join(" ") },
        Some("snapshots") => Request::ListSnapshots,
        Some("rewind") => Request::Rewind { id: arg(1) },
        Some("auto-snapshot") => Request::SetAutoSnapshot { on: arg(1) == "on" },
        Some("jack-memory") => Request::SetJackMemory { on: arg(1) == "on" },
        Some("rules" | "rule-add" | "rule-del") => Request::Rules,
        Some("props") => Request::State,
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
    // props: the client's properties now, changed by the words given, sent back.
    let req = if a[0] == "props" {
        let id = arg(1);
        let Ok(Reply::State(st)) = c.call(&Request::State) else {
            eprintln!("patchbox-ctl: no state from patchboxd");
            std::process::exit(1);
        };
        let Some(cl) = st.clients.iter().find(|x| x.id == id) else {
            eprintln!("error: no client {id}");
            std::process::exit(1);
        };
        let mut p = pbx_session::proto::ClientProps {
            label: cl.label.clone(),
            exe: cl.exe.clone(),
            args: cl.args.clone(),
            env: cl.env.clone(),
            save_signal: cl.save_signal,
            stop_signal: cl.stop_signal,
            terminal: cl.terminal,
        };
        let sig = |v: &str| -> i32 {
            let n = v.trim_start_matches("SIG");
            n.parse().unwrap_or(match n {
                "USR1" => libc::SIGUSR1,
                "USR2" => libc::SIGUSR2,
                "HUP" => libc::SIGHUP,
                "INT" => libc::SIGINT,
                "TERM" => libc::SIGTERM,
                "QUIT" => libc::SIGQUIT,
                _ => 0,
            })
        };
        let mut words = a[2..].iter();
        let mut env_given = false;
        while let Some(w) = words.next() {
            if w == "--" {
                let rest: Vec<String> = words.by_ref().cloned().collect();
                if let Some((exe, args)) = rest.split_first() {
                    p.exe = exe.clone();
                    p.args = args.to_vec();
                }
                break;
            }
            match w.split_once('=') {
                Some(("label", v)) => p.label = v.into(),
                Some(("save", v)) => p.save_signal = sig(v),
                Some(("stop", v)) => p.stop_signal = sig(v),
                Some(("terminal", v)) => p.terminal = v == "on",
                Some(("env", kv)) => {
                    if !env_given {
                        p.env.clear();
                        env_given = true;
                    }
                    if let Some((k, v)) = kv.split_once('=') {
                        p.env.push((k.into(), v.into()));
                    }
                }
                _ => usage(),
            }
        }
        Request::SetClientProps { id, props: p }
    } else {
        req
    };
    // rule-add / rule-del: the rules, changed, sent back.
    let req = if matches!(a[0].as_str(), "rule-add" | "rule-del") {
        let mut rules = match c.call(&Request::Rules) {
            Ok(Reply::Rules { rules }) => rules,
            Ok(Reply::Error { msg }) => {
                eprintln!("error: {msg}");
                std::process::exit(1);
            }
            _ => {
                eprintln!("patchbox-ctl: no rules from patchboxd");
                std::process::exit(1);
            }
        };
        let r = pbx_session::rules::Rule { out: arg(1), inp: arg(2) };
        if a[0] == "rule-add" {
            rules.push(r);
        } else {
            rules.retain(|x| *x != r);
        }
        Request::SetRules { rules }
    } else {
        req
    };
    match c.call(&req) {
        Ok(Reply::Ok { msg }) => println!("{msg}"),
        Ok(Reply::Error { msg }) => {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
        Ok(Reply::Sessions { root, sessions, recent }) => {
            println!("# {root}");
            if !recent.is_empty() {
                println!("# recent: {}", recent.join(", "));
            }
            for s in sessions {
                println!("{}\t{} clients\t{}", s.name, s.clients, s.notes);
            }
        }
        Ok(Reply::Templates { templates }) => {
            for t in templates {
                println!("{t}");
            }
        }
        Ok(Reply::Snapshots { snapshots }) => {
            for s in snapshots {
                println!("{}\t{}\t{}", s.id, s.time, s.label);
            }
        }
        Ok(Reply::State(st)) => println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default()),
        Ok(Reply::Rules { rules }) => {
            for r in rules {
                println!("{}\t{}", r.out, r.inp);
            }
        }
        Err(e) => {
            eprintln!("patchbox-ctl: {e}");
            std::process::exit(1);
        }
    }
    if !watch {
        return;
    }
    loop {
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
