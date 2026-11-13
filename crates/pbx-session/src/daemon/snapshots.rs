//! Snapshots of a session folder: a git repository in `.patchbox-snapshots/` with the session
//! folder as its work tree (RaySession's scheme). Audio and video files, files over 50 MB, our
//! logs and the lock file are left out (regenerated `info/exclude`). Each snapshot is a commit
//! with a tag `pb-<UTC time>`; going back is `reset --hard` to a tag (the other tags keep the
//! later snapshots).
//!
//! git runs as a child of the daemon (reaped like the session's clients), never blocking the
//! loop. It runs with signing and hooks off and an identity of its own: the user's git
//! configuration (commit.gpgsign, hooks) must not apply to these commits.

use crate::proto::SnapshotEntry;
use std::path::Path;
use std::process::{Command, Stdio};

pub const DIR: &str = ".patchbox-snapshots";

const PRELUDE: &str = r#"
D="$1"
g() { git --git-dir="$D/.patchbox-snapshots" --work-tree="$D" \
  -c commit.gpgsign=false -c tag.gpgsign=false -c core.hooksPath=/dev/null \
  -c user.name=PatchBox -c user.email=patchbox@localhost -c init.defaultBranch=main "$@"; }
"#;

const SNAPSHOT: &str = r#"
set -e
[ -d "$D/.patchbox-snapshots" ] || g init -q
{
  echo '/.patchbox-snapshots/'; echo '/.patchbox/'; echo '/.patchbox.lock'; echo '/.*.tmp'
  for e in wav WAV flac ogg mp3 mp4 avi mkv peak m4a pdf; do echo "*.$e"; done
  find "$D" -path "$D/.patchbox-snapshots" -prune -o -type f -size +50M -printf '/%P\n'
} > "$D/.patchbox-snapshots/info/exclude"
g add -A
if [ "$3" = auto ] && g rev-parse -q --verify HEAD >/dev/null && g diff --cached --quiet; then
  exit 0
fi
g commit -q --allow-empty -m "$2"
g tag "pb-$(date -u +%Y%m%d-%H%M%S-%N | cut -c1-19)"
"#;

const CHECKOUT: &str = r#"
set -e
g reset -q --hard "$2"
"#;

fn spawn(dir: &Path, script: &str, a2: &str, a3: &str) -> Result<u32, String> {
    let log = dir.join(".patchbox");
    let _ = std::fs::create_dir_all(&log);
    let out = std::fs::OpenOptions::new().create(true).append(true).open(log.join("git.log")).ok();
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!("{PRELUDE}{script}")).arg("sh").arg(dir).arg(a2).arg(a3).current_dir(dir).stdin(Stdio::null());
    match out.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
        Some((a, b)) => {
            cmd.stdout(a).stderr(b);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGCHLD);
            libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
    Ok(child.id())
}

/// Start a snapshot; `auto`: only if something changed since the last one.
pub fn spawn_snapshot(dir: &Path, label: &str, auto: bool) -> Result<u32, String> {
    spawn(dir, SNAPSHOT, label, if auto { "auto" } else { "named" })
}

pub fn spawn_checkout(dir: &Path, tag: &str) -> Result<u32, String> {
    if !tag.starts_with("pb-") || tag.contains(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        return Err(format!("not a snapshot: {tag}"));
    }
    spawn(dir, CHECKOUT, tag, "")
}

/// Snapshots, newest first (quick: reads refs).
pub fn list(dir: &Path) -> Vec<SnapshotEntry> {
    if !dir.join(DIR).is_dir() {
        return Vec::new();
    }
    let out = Command::new("git")
        .arg(format!("--git-dir={}", dir.join(DIR).display()))
        .args(["for-each-ref", "--sort=-creatordate", "--format=%(refname:short)%09%(creatordate:unix)%09%(subject)", "refs/tags/pb-*"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(o) = out else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some(SnapshotEntry { id: it.next()?.to_string(), time: it.next()?.parse().ok()?, label: it.next().unwrap_or("").to_string() })
        })
        .collect()
}
