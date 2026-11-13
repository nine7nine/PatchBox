//! PatchBox's session manager: patchboxd (an NSM server that launches and supervises the
//! session's clients and remembers its JACK connections), the session format, and the control
//! protocol the GUI and patchbox-ctl speak to it.

pub mod daemon;
pub mod jackcfg;
pub mod osc;
pub mod proto;
pub mod rules;
pub mod store;
pub mod trash;
