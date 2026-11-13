//! The desktop's trash (the freedesktop.org Trash specification, home trash only): a deleted
//! session's folder is moved there whole, with the .trashinfo file managers use to put it back.
//! Nothing is ever removed for good here.

use std::io::Write;
use std::path::{Path, PathBuf};

/// $XDG_DATA_HOME/Trash, else ~/.local/share/Trash.
pub fn home_trash() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("Trash"))
}

/// Move `path` (absolute) into the home trash; returns where it went.
pub fn to_trash(path: &Path) -> Result<PathBuf, String> {
    let trash = home_trash().ok_or("no home folder for the Trash")?;
    to_trash_in(&trash, path)
}

/// Move `path` into the trash at `trash`. The .trashinfo is created first (exclusively: it
/// reserves the name), then the folder is renamed in; a rename that fails (another filesystem)
/// takes the .trashinfo back and leaves `path` where it was.
pub fn to_trash_in(trash: &Path, path: &Path) -> Result<PathBuf, String> {
    let files = trash.join("files");
    let info = trash.join("info");
    for d in [&files, &info] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let base = path.file_name().ok_or("nothing to trash")?.to_string_lossy().into_owned();
    let text = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", encode_path(path), local_time());
    for n in 1..1000 {
        let name = if n == 1 { base.clone() } else { format!("{base}.{n}") };
        let to = files.join(&name);
        let info_file = info.join(format!("{name}.trashinfo"));
        if to.symlink_metadata().is_ok() {
            continue;
        }
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&info_file) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", info_file.display())),
        };
        if let Err(e) = f.write_all(text.as_bytes()) {
            let _ = std::fs::remove_file(&info_file);
            return Err(format!("{}: {e}", info_file.display()));
        }
        drop(f);
        return match std::fs::rename(path, &to) {
            Ok(()) => Ok(to),
            Err(e) => {
                let _ = std::fs::remove_file(&info_file);
                Err(if e.raw_os_error() == Some(libc::EXDEV) {
                    format!("{} is on another filesystem than the Trash; left as it is", path.display())
                } else {
                    format!("{}: {e}", path.display())
                })
            }
        };
    }
    Err("the Trash has too many items of that name".into())
}

/// The spec's Path=: the absolute path, percent-encoded (RFC 2396) but for '/'.
fn encode_path(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/!*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The spec's DeletionDate=: local time, YYYY-MM-DDThh:mm:ss.
fn local_time() -> String {
    // SAFETY: time() and localtime_r() with valid pointers; `tm` is plain data.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_with_info_and_keeps_names_apart() {
        let d = std::env::temp_dir().join(format!("pbx-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (root, trash) = (d.join("sessions"), d.join("Trash"));
        for _ in 0..2 {
            let s = root.join("live/gig 1");
            std::fs::create_dir_all(s.join("Ardour.nABC")).unwrap();
            std::fs::write(s.join("session.toml"), "x").unwrap();
            to_trash_in(&trash, &s).unwrap();
            assert!(!s.exists());
        }
        assert!(trash.join("files/gig 1/session.toml").is_file());
        assert!(trash.join("files/gig 1.2/Ardour.nABC").is_dir());
        let info = std::fs::read_to_string(trash.join("info/gig 1.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("/sessions/live/gig%201\n"), "{info}");
        assert!(info.contains("\nDeletionDate=2"), "{info}");
        assert!(trash.join("info/gig 1.2.trashinfo").is_file());
        // Nothing there: an error, and no .trashinfo left behind.
        assert!(to_trash_in(&trash, &root.join("none")).is_err());
        assert!(!trash.join("info/none.trashinfo").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
