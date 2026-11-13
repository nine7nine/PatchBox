//! The NSM server side (Non Session Manager API 1.1): what clients send us over OSC.

use super::Daemon;
use super::clients::Client;
use crate::osc::{self, Arg, Msg};
use crate::proto::{Request, Status};
use crate::store::{self, ClientEntry};
use std::net::SocketAddr;

// NSM error codes.
const ERR_GENERAL: i32 = -1;
const ERR_INCOMPATIBLE_API: i32 = -2;
const ERR_NO_SESSION_OPEN: i32 = -6;

/// The parent of a process, from /proc.
fn ppid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // "pid (comm) state ppid …": comm may hold spaces and parentheses; take the last ')'.
    let rest = &s[s.rfind(')')? + 2..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn descends_from(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..16 {
        match ppid(pid) {
            Some(p) if p == ancestor => return true,
            Some(p) if p > 1 => pid = p,
            _ => return false,
        }
    }
    false
}

impl Daemon {
    fn reply_to(&self, addr: SocketAddr, m: &Msg) {
        let _ = self.udp.send_to(&osc::encode(m), addr);
    }

    fn error_to(&self, addr: SocketAddr, path: &str, code: i32, msg: &str) {
        self.reply_to(addr, &Msg::new("/error", vec![Arg::S(path.into()), Arg::I(code), Arg::S(msg.into())]));
    }

    pub(super) fn read_udp(&mut self) {
        let mut buf = vec![0u8; 65536];
        while let Ok((n, addr)) = self.udp.recv_from(&mut buf) {
            for m in osc::decode(&buf[..n]) {
                self.nsm(m, addr);
            }
        }
    }

    fn client_at(&self, addr: SocketAddr) -> Option<usize> {
        self.session.as_ref()?.clients.iter().position(|c| c.out.as_ref().is_some_and(|o| o.1 == addr))
    }

    fn nsm(&mut self, m: Msg, addr: SocketAddr) {
        match m.path.as_str() {
            "/nsm/server/announce" => self.announce(&m, addr),
            "/reply" | "/error" => {
                let Some(i) = self.client_at(addr) else { return };
                let to = m.s(0).unwrap_or("").to_string();
                let (ok, text) = if m.path == "/reply" { (true, m.s(1).unwrap_or("")) } else { (false, m.s(2).unwrap_or("")) };
                let text = text.to_string();
                let line = self.session.as_mut().unwrap().clients[i].replied(&to, ok, &text);
                if let Some((l, bad)) = line {
                    self.log(l, bad);
                }
            }
            "/nsm/client/is_dirty" | "/nsm/client/is_clean" => {
                if let Some(i) = self.client_at(addr) {
                    self.session.as_mut().unwrap().clients[i].dirty = Some(m.path.ends_with("is_dirty"));
                }
            }
            "/nsm/client/progress" => {
                if let Some(i) = self.client_at(addr) {
                    self.session.as_mut().unwrap().clients[i].progress = m.f(0).unwrap_or(0.0) as f32;
                }
            }
            "/nsm/client/message" => {
                if let Some(i) = self.client_at(addr) {
                    let text = m.s(1).unwrap_or("").to_string();
                    let c = &mut self.session.as_mut().unwrap().clients[i];
                    c.message = text.clone();
                    let id = c.entry.id.clone();
                    self.log(format!("{id}: {text}"), m.i(0).unwrap_or(0) > 1);
                }
            }
            "/nsm/client/label" => {
                if let Some(i) = self.client_at(addr) {
                    self.session.as_mut().unwrap().clients[i].entry.label = m.s(0).unwrap_or("").to_string();
                }
            }
            "/nsm/client/gui_is_shown" | "/nsm/client/gui_is_hidden" => {
                if let Some(i) = self.client_at(addr) {
                    let c = &mut self.session.as_mut().unwrap().clients[i];
                    let shown = m.path.ends_with("shown");
                    c.gui = Some(shown);
                    c.entry.gui_visible = Some(shown);
                }
            }
            "/nsm/server/broadcast" => {
                // Relay to every other NSM client; never our own namespace.
                let Some(path) = m.s(0).map(str::to_string) else { return };
                if path.starts_with("/nsm/") {
                    return;
                }
                let relayed = Msg::new(&path, m.args[1..].to_vec());
                if let Some(s) = &self.session {
                    for c in &s.clients {
                        if c.out.as_ref().is_some_and(|o| o.1 != addr) {
                            c.send(&relayed);
                        }
                    }
                }
            }
            p if p.starts_with("/nsm/server/") => self.server_control(&m, addr),
            _ => self.error_to(addr, &m.path, ERR_GENERAL, "unknown message"),
        }
    }

    fn announce(&mut self, m: &Msg, addr: SocketAddr) {
        let path = "/nsm/server/announce";
        let (Some(name), Some(caps), Some(exe)) = (m.s(0), m.s(1), m.s(2)) else {
            return self.error_to(addr, path, ERR_GENERAL, "bad announce");
        };
        let (major, pid) = (m.i(3).unwrap_or(1), m.i(5).unwrap_or(0) as u32);
        if major > 1 {
            return self.error_to(addr, path, ERR_INCOMPATIBLE_API, "incompatible API version");
        }
        let Some(s) = &mut self.session else {
            return self.error_to(addr, path, ERR_NO_SESSION_OPEN, "no session open");
        };
        // Which of our clients is it: its pid, a process it started (wrapper scripts), else one
        // we launched with that executable that hasn't announced yet.
        let waiting = |c: &Client| c.out.is_none() && c.running();
        let base = |e: &str| std::path::Path::new(e).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let found = s
            .clients
            .iter()
            .position(|c| c.pid == Some(pid) && pid != 0)
            .or_else(|| s.clients.iter().position(|c| waiting(c) && c.pid.is_some_and(|p| pid != 0 && descends_from(pid, p))))
            .or_else(|| s.clients.iter().position(|c| waiting(c) && base(&c.entry.exe) == base(exe)));
        let i = match found {
            Some(i) => i,
            None => {
                // Started by hand with our NSM_URL: it joins the session.
                let taken: Vec<String> = s.clients.iter().map(|c| c.entry.id.clone()).collect();
                let id = store::client_id(exe, &taken);
                let entry = ClientEntry { id, exe: exe.to_string(), nsm: true, name: name.to_string(), autostart: true, ..Default::default() };
                let mut c = Client::new(entry);
                c.pid = (pid != 0).then_some(pid);
                c.external = true;
                c.late = s.loaded;
                s.clients.push(c);
                s.clients.len() - 1
            }
        };
        let Ok(sock) = self.udp.try_clone() else { return };
        let (dir, sname) = (s.dir.clone(), s.name.clone());
        let c = &mut s.clients[i];
        if c.pid.is_none() || c.external {
            c.pid = (pid != 0).then_some(pid);
        }
        if c.status == Status::NoAnnounce {
            c.message.clear();
        }
        c.late |= s.loaded;
        c.announced((sock, addr), name, caps, &dir, &sname);
        let line = format!("{} announced as {name} ({})", c.entry.id, if caps.is_empty() { "no capabilities" } else { caps });
        self.log(line, false);
    }

    /// Clients driving the server (:server-control:).
    fn server_control(&mut self, m: &Msg, addr: SocketAddr) {
        let path = m.path.clone();
        let arg = m.s(0).unwrap_or("").to_string();
        let req = match path.as_str() {
            "/nsm/server/save" => Request::Save,
            "/nsm/server/close" => Request::Close,
            "/nsm/server/abort" => Request::Abort,
            "/nsm/server/quit" => Request::Quit,
            "/nsm/server/open" => Request::Open { name: arg },
            "/nsm/server/new" => Request::New { name: arg },
            "/nsm/server/duplicate" => Request::Duplicate { name: arg },
            "/nsm/server/add" => {
                if arg.contains('/') {
                    return self.error_to(addr, &path, ERR_GENERAL, "absolute paths are not allowed");
                }
                Request::Add { exe: arg, args: Vec::new(), nsm: true, label: String::new() }
            }
            "/nsm/server/list" => {
                for e in store::list(&self.root_dir()) {
                    self.reply_to(addr, &Msg::new("/reply", vec![Arg::S(path.clone()), Arg::S(e.name)]));
                }
                self.reply_to(addr, &Msg::new("/reply", vec![Arg::S(path), Arg::S(String::new())]));
                return;
            }
            _ => return self.error_to(addr, &path, ERR_GENERAL, "unknown server command"),
        };
        match self.request_from_nsm(req) {
            Ok(msg) => self.reply_to(addr, &Msg::new("/reply", vec![Arg::S(path), Arg::S(msg)])),
            Err(msg) => self.error_to(addr, &path, ERR_GENERAL, &msg),
        }
    }
}
