//! Client templates (RaySession's): a client kept with its command, properties and files, to add
//! to any session with its own copy. Its files are those in the session folder named after it
//! (`<name>.<id>…`, the path NSM gave it); in the template that prefix is `__CLIENT__`, and a
//! client added from it gets its own.
//!
//! Per-client hooks: `patchbox-scripts/clients/<id>/<start|save|stop>.sh`, else
//! `patchbox-scripts/clients/<start|save|stop>.sh`, in the session's folder or one above it
//! (within the sessions root), run beside the client when it starts, after it saved, after it
//! stopped; they don't hold it up. They get the session's PATCHBOX_* variables and
//! PATCHBOX_CLIENT_ID, PATCHBOX_CLIENT_PID, PATCHBOX_CLIENT_JACK_NAME.

use super::{Daemon, copy_dir};
use crate::daemon::clients::Client;
use crate::proto::{self, Reply};
use crate::store::{self, ClientEntry};
use std::path::{Path, PathBuf};

const MARK: &str = "__CLIENT__";

/// A client's file prefix in its session folder.
pub fn prefix(e: &ClientEntry) -> String {
    let name: String = e.name.chars().map(|c| if c == '/' { '_' } else { c }).collect();
    format!("{name}.{}", e.id)
}

fn copy_renamed(from: &Path, to: &Path, old: &str, new: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let fname = e.file_name().to_string_lossy().into_owned();
        if !fname.starts_with(old) {
            continue;
        }
        let dst = to.join(fname.replacen(old, new, 1));
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dst, &[])?;
        } else {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

impl Daemon {
    pub(crate) fn save_client_template(&mut self, id: &str, name: &str) -> Reply {
        let name = name.trim();
        if store::valid_name(name).is_err() || name.contains('/') {
            return Reply::Error { msg: format!("\"{name}\" can't be a template's name") };
        }
        let Some(s) = &self.session else { return Reply::Error { msg: "no session is open".into() } };
        let Some(c) = s.clients.iter().find(|c| c.entry.id == id) else { return Reply::Error { msg: format!("no client {id}") } };
        let Some(root) = proto::client_templates_root() else { return Reply::Error { msg: "no configuration directory".into() } };
        let dst = root.join(name);
        let _ = std::fs::remove_dir_all(&dst);
        let mut e = c.to_entry();
        let files = copy_renamed(&s.dir, &dst.join("files"), &prefix(&e), MARK);
        e.last_save = None;
        e.autostart = true;
        let body = match toml::to_string_pretty(&e) {
            Ok(b) => b,
            Err(err) => return Reply::Error { msg: err.to_string() },
        };
        match files.and_then(|_| std::fs::write(dst.join("client.toml"), body)) {
            Ok(()) => Reply::Ok { msg: format!("{id} kept as the client template {name}") },
            Err(err) => Reply::Error { msg: format!("client template {name}: {err}") },
        }
    }

    pub(crate) fn list_client_templates(&self) -> Reply {
        let mut t: Vec<String> = proto::client_templates_root()
            .and_then(|r| std::fs::read_dir(r).ok())
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().join("client.toml").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        t.sort();
        Reply::ClientTemplates { templates: t }
    }

    pub(crate) fn add_client_template(&mut self, name: &str) -> Reply {
        let Some(dir) = proto::client_templates_root().map(|r| r.join(name)) else { return Reply::Error { msg: "no configuration directory".into() } };
        let Ok(text) = std::fs::read_to_string(dir.join("client.toml")) else { return Reply::Error { msg: format!("no client template {name}") } };
        let mut e: ClientEntry = match toml::from_str(&text) {
            Ok(e) => e,
            Err(err) => return Reply::Error { msg: format!("client template {name}: {err}") },
        };
        let Some(s) = &mut self.session else { return Reply::Error { msg: "open or create a session first".into() } };
        let mut taken: Vec<String> = s.clients.iter().map(|c| c.entry.id.clone()).chain(s.trash.iter().map(|e| e.id.clone())).collect();
        if let Ok(rd) = std::fs::read_dir(&s.dir) {
            taken.extend(rd.filter_map(|x| x.ok()).map(|x| x.file_name().to_string_lossy().into_owned()));
        }
        e.id = store::client_id(&e.exe, &taken);
        if let Err(err) = copy_renamed(&dir.join("files"), &s.dir, MARK, &prefix(&e)) {
            return Reply::Error { msg: format!("client template {name}: {err}") };
        }
        let id = e.id.clone();
        s.clients.push(Client::new(e));
        let i = s.clients.len() - 1;
        match self.launch(i) {
            Ok(_) => Reply::Ok { msg: format!("added {id} from the client template {name}") },
            Err(err) => Reply::Error { msg: err },
        }
    }

    /// Run a client's hook (`start`, `save`, `stop`) if the session has one.
    pub(crate) fn client_hook(&self, which: &str, c: &Client) {
        let Some(s) = &self.session else { return };
        let mut d = Some(s.dir.as_path());
        let script: PathBuf = loop {
            let Some(here) = d else { return };
            let base = here.join("patchbox-scripts").join("clients");
            let own = base.join(&c.entry.id).join(format!("{which}.sh"));
            let all = base.join(format!("{which}.sh"));
            if own.is_file() {
                break own;
            }
            if all.is_file() {
                break all;
            }
            if here == self.root {
                return;
            }
            d = here.parent().filter(|p| p.starts_with(&self.root));
        };
        let logs = s.dir.join(".patchbox");
        super::clients::rotate(&logs.join("scripts.log"), 256 << 10);
        let log = std::fs::OpenOptions::new().create(true).append(true).open(logs.join("scripts.log")).ok();
        let mut cmd = std::process::Command::new("sh");
        cmd.arg(&script)
            .current_dir(&s.dir)
            .env("PATCHBOX_SCRIPT", format!("client-{which}"))
            .env("PATCHBOX_SESSION_NAME", &s.name)
            .env("PATCHBOX_SESSION_PATH", &s.dir)
            .env("PATCHBOX_SOCKET", proto::socket_path())
            .env("PATCHBOX_CLIENT_ID", &c.entry.id)
            .env("PATCHBOX_CLIENT_PID", c.pid.map(|p| p.to_string()).unwrap_or_default())
            .env("PATCHBOX_CLIENT_JACK_NAME", &c.jack_name)
            .stdin(std::process::Stdio::null());
        if let Some((a, b)) = log.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
            cmd.stdout(a).stderr(b);
        }
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        if let Err(err) = cmd.spawn() {
            eprintln!("[patchboxd] {}: {err}", script.display());
        }
    }
}
