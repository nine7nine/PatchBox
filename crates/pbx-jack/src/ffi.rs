//! The part of libjack's C API PatchBox uses (jack/jack.h, jack/metadata.h, jack/uuid.h).
//!
//! Linked against the system libjack (jack2: libjack.so.0). Only non-realtime calls: the patchbay
//! client registers no process callback, so jack2 starts no RT thread for it and never inserts it
//! in the graph's activation chain (JackClient::Activate, JackEngine::ClientActivate).

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_float, c_int, c_ulong, c_void};

#[repr(C)]
pub struct jack_client_t {
    _private: [u8; 0],
}
/// A port's latency, frames (jack_latency_range_t).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct jack_latency_range_t {
    pub min: u32,
    pub max: u32,
}

pub const JACK_CAPTURE_LATENCY: c_int = 0;
pub const JACK_PLAYBACK_LATENCY: c_int = 1;

#[repr(C)]
pub struct jack_port_t {
    _private: [u8; 0],
}

pub type jack_nframes_t = u32;
pub type jack_port_id_t = u32;
pub type jack_uuid_t = u64;
pub type jack_options_t = c_int;
pub type jack_status_t = c_int;
pub type jack_property_change_t = c_int;

// jack_options_t
pub const JACK_NULL_OPTION: jack_options_t = 0x00;
pub const JACK_NO_START_SERVER: jack_options_t = 0x01;
pub const JACK_USE_EXACT_NAME: jack_options_t = 0x02;
pub const JACK_SERVER_NAME: jack_options_t = 0x04;

// jack_status_t
pub const JACK_FAILURE: jack_status_t = 0x01;
pub const JACK_INVALID_OPTION: jack_status_t = 0x02;
pub const JACK_NAME_NOT_UNIQUE: jack_status_t = 0x04;
pub const JACK_SERVER_STARTED: jack_status_t = 0x08;
pub const JACK_SERVER_FAILED: jack_status_t = 0x10;
pub const JACK_SERVER_ERROR: jack_status_t = 0x20;
pub const JACK_NO_SUCH_CLIENT: jack_status_t = 0x40;
pub const JACK_LOAD_FAILURE: jack_status_t = 0x80;
pub const JACK_INIT_FAILURE: jack_status_t = 0x100;
pub const JACK_SHM_FAILURE: jack_status_t = 0x200;
pub const JACK_VERSION_ERROR: jack_status_t = 0x400;

// JackPortFlags
pub const JACK_PORT_IS_INPUT: u32 = 0x1;
pub const JACK_PORT_IS_OUTPUT: u32 = 0x2;
pub const JACK_PORT_IS_PHYSICAL: u32 = 0x4;
pub const JACK_PORT_CAN_MONITOR: u32 = 0x8;
pub const JACK_PORT_IS_TERMINAL: u32 = 0x10;

// jack_property_change_t
pub const PROPERTY_CREATED: jack_property_change_t = 0;
pub const PROPERTY_CHANGED: jack_property_change_t = 1;
pub const PROPERTY_DELETED: jack_property_change_t = 2;

pub const DEFAULT_AUDIO_TYPE: &str = "32 bit float mono audio";
pub const DEFAULT_MIDI_TYPE: &str = "8 bit raw midi";

pub type JackInfoShutdownCallback = extern "C" fn(code: jack_status_t, reason: *const c_char, arg: *mut c_void);
pub type JackClientRegistrationCallback = extern "C" fn(name: *const c_char, register: c_int, arg: *mut c_void);
pub type JackPortRegistrationCallback = extern "C" fn(port: jack_port_id_t, register: c_int, arg: *mut c_void);
pub type JackPortConnectCallback = extern "C" fn(a: jack_port_id_t, b: jack_port_id_t, connect: c_int, arg: *mut c_void);
pub type JackPortRenameCallback =
    extern "C" fn(port: jack_port_id_t, old_name: *const c_char, new_name: *const c_char, arg: *mut c_void);
pub type JackGraphOrderCallback = extern "C" fn(arg: *mut c_void) -> c_int;
pub type JackXRunCallback = extern "C" fn(arg: *mut c_void) -> c_int;
pub type JackBufferSizeCallback = extern "C" fn(nframes: jack_nframes_t, arg: *mut c_void) -> c_int;
pub type JackSampleRateCallback = extern "C" fn(nframes: jack_nframes_t, arg: *mut c_void) -> c_int;
pub type JackFreewheelCallback = extern "C" fn(starting: c_int, arg: *mut c_void);
pub type JackPropertyChangeCallback =
    extern "C" fn(subject: jack_uuid_t, key: *const c_char, change: jack_property_change_t, arg: *mut c_void);
pub type JackErrorFn = extern "C" fn(msg: *const c_char);

#[link(name = "jack")]
unsafe extern "C" {
    pub fn jack_client_open(name: *const c_char, options: jack_options_t, status: *mut jack_status_t, ...) -> *mut jack_client_t;
    pub fn jack_client_close(client: *mut jack_client_t) -> c_int;
    pub fn jack_activate(client: *mut jack_client_t) -> c_int;
    pub fn jack_deactivate(client: *mut jack_client_t) -> c_int;
    pub fn jack_get_client_name(client: *mut jack_client_t) -> *const c_char;
    pub fn jack_get_uuid_for_client_name(client: *mut jack_client_t, name: *const c_char) -> *mut c_char;
    pub fn jack_get_client_name_by_uuid(client: *mut jack_client_t, uuid: *const c_char) -> *mut c_char;
    pub fn jack_client_get_uuid(client: *mut jack_client_t) -> *mut c_char;
    pub fn jack_uuid_parse(buf: *const c_char, uuid: *mut jack_uuid_t) -> c_int;
    pub fn jack_free(ptr: *mut c_void);

    pub fn jack_set_error_function(f: JackErrorFn);
    pub fn jack_set_info_function(f: JackErrorFn);

    pub fn jack_on_info_shutdown(client: *mut jack_client_t, cb: JackInfoShutdownCallback, arg: *mut c_void);
    pub fn jack_set_client_registration_callback(client: *mut jack_client_t, cb: JackClientRegistrationCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_port_registration_callback(client: *mut jack_client_t, cb: JackPortRegistrationCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_port_connect_callback(client: *mut jack_client_t, cb: JackPortConnectCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_port_rename_callback(client: *mut jack_client_t, cb: JackPortRenameCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_graph_order_callback(client: *mut jack_client_t, cb: JackGraphOrderCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_xrun_callback(client: *mut jack_client_t, cb: JackXRunCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_buffer_size_callback(client: *mut jack_client_t, cb: JackBufferSizeCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_sample_rate_callback(client: *mut jack_client_t, cb: JackSampleRateCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_freewheel_callback(client: *mut jack_client_t, cb: JackFreewheelCallback, arg: *mut c_void) -> c_int;
    pub fn jack_set_property_change_callback(client: *mut jack_client_t, cb: JackPropertyChangeCallback, arg: *mut c_void) -> c_int;

    pub fn jack_get_ports(client: *mut jack_client_t, port_name_pattern: *const c_char, type_name_pattern: *const c_char, flags: c_ulong)
    -> *mut *const c_char;
    pub fn jack_port_by_name(client: *mut jack_client_t, name: *const c_char) -> *mut jack_port_t;
    pub fn jack_port_by_id(client: *mut jack_client_t, id: jack_port_id_t) -> *mut jack_port_t;
    pub fn jack_port_name(port: *const jack_port_t) -> *const c_char;
    /// `mode`: JACK_CAPTURE_LATENCY or JACK_PLAYBACK_LATENCY.
    pub fn jack_port_get_latency_range(port: *mut jack_port_t, mode: c_int, range: *mut jack_latency_range_t);
    pub fn jack_port_short_name(port: *const jack_port_t) -> *const c_char;
    pub fn jack_port_flags(port: *const jack_port_t) -> c_int;
    pub fn jack_port_type(port: *const jack_port_t) -> *const c_char;
    pub fn jack_port_uuid(port: *const jack_port_t) -> jack_uuid_t;
    pub fn jack_port_get_aliases(port: *const jack_port_t, aliases: *mut *mut c_char) -> c_int;
    pub fn jack_port_name_size() -> c_int;
    pub fn jack_port_get_all_connections(client: *const jack_client_t, port: *const jack_port_t) -> *mut *const c_char;
    pub fn jack_port_register(client: *mut jack_client_t, name: *const c_char, port_type: *const c_char, flags: c_ulong, buffer_size: c_ulong)
    -> *mut jack_port_t;

    pub fn jack_connect(client: *mut jack_client_t, source: *const c_char, destination: *const c_char) -> c_int;
    pub fn jack_disconnect(client: *mut jack_client_t, source: *const c_char, destination: *const c_char) -> c_int;

    pub fn jack_get_sample_rate(client: *mut jack_client_t) -> jack_nframes_t;
    pub fn jack_get_buffer_size(client: *mut jack_client_t) -> jack_nframes_t;
    pub fn jack_set_buffer_size(client: *mut jack_client_t, nframes: jack_nframes_t) -> c_int;
    pub fn jack_cpu_load(client: *mut jack_client_t) -> c_float;
    /// How late the last xrun was, microseconds.
    pub fn jack_get_xrun_delayed_usecs(client: *mut jack_client_t) -> c_float;
    pub fn jack_is_realtime(client: *mut jack_client_t) -> c_int;
    pub fn jack_set_freewheel(client: *mut jack_client_t, onoff: c_int) -> c_int;

    /// `pos`: a jack_position_t (136 bytes, packed; offsets in lib.rs).
    pub fn jack_transport_query(client: *const jack_client_t, pos: *mut u8) -> c_int;
    pub fn jack_transport_start(client: *mut jack_client_t);
    pub fn jack_transport_stop(client: *mut jack_client_t);
    pub fn jack_transport_locate(client: *mut jack_client_t, frame: jack_nframes_t) -> c_int;

    pub fn jack_get_property(subject: jack_uuid_t, key: *const c_char, value: *mut *mut c_char, type_: *mut *mut c_char) -> c_int;
    pub fn jack_set_property(client: *mut jack_client_t, subject: jack_uuid_t, key: *const c_char, value: *const c_char, type_: *const c_char)
    -> c_int;
    pub fn jack_remove_property(client: *mut jack_client_t, subject: jack_uuid_t, key: *const c_char) -> c_int;
}
