//! The session daemon. Started by the GUI (or by hand); exits when idle with no session.
fn main() {
    if let Err(e) = pbx_session::daemon::run() {
        eprintln!("patchboxd: {e}");
        std::process::exit(1);
    }
}
