//! Raw bindings to native/psnative.h. Keep layouts in sync with that header.
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_void};

pub type LogCb = unsafe extern "C" fn(level: c_int, msg: *const c_char, user: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PsHost {
    pub state: i32,
    pub is_ps5: i32,
    pub request_port: u16,
    pub host_addr: [c_char; 64],
    pub host_name: [c_char; 128],
    pub host_id: [c_char; 64],
    pub system_version: [c_char; 32],
    pub running_app_name: [c_char; 128],
    pub running_app_titleid: [c_char; 32],
}

pub type DiscoveryCb = unsafe extern "C" fn(hosts: *const PsHost, count: usize, user: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PsRegisteredHost {
    pub target: i32,
    pub server_nickname: [c_char; 32],
    pub server_mac: [u8; 6],
    pub rp_regist_key: [c_char; 16],
    pub rp_key: [u8; 16],
    pub rp_key_type: u32,
    pub console_pin: u32,
}

pub type RegistCb =
    unsafe extern "C" fn(result: c_int, host: *const PsRegisteredHost, user: *mut c_void);

#[repr(C)]
pub struct PsSessionConfig {
    pub host: *const c_char,
    pub ps5: i32,
    pub regist_key: [c_char; 16],
    pub morning: [u8; 16],
    pub psn_account_id: [u8; 8],
    pub resolution: i32,
    pub fps: i32,
    pub codec: i32,
    pub bitrate: u32,
    pub hw_decode: i32,
}

pub const EVENT_CONNECTED: c_int = 0;
pub const EVENT_QUIT: c_int = 1;
pub const EVENT_RUMBLE: c_int = 2;
pub const EVENT_LOGIN_PIN_REQUEST: c_int = 3;
pub const EVENT_NICKNAME: c_int = 4;

#[repr(C)]
pub struct PsSessionCallbacks {
    pub event: Option<
        unsafe extern "C" fn(ty: c_int, a: c_int, b: c_int, s: *const c_char, user: *mut c_void),
    >,
    pub audio_settings: Option<unsafe extern "C" fn(channels: u32, rate: u32, user: *mut c_void)>,
    pub audio_frame:
        Option<unsafe extern "C" fn(samples: *const i16, frames: usize, user: *mut c_void)>,
    pub frame_available: Option<unsafe extern "C" fn(user: *mut c_void)>,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PsController {
    pub buttons: u32,
    pub l2: u8,
    pub r2: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
}

pub const PIXFMT_NV12: i32 = 0;
pub const PIXFMT_YUV420P: i32 = 1;
pub const PIXFMT_P010: i32 = 2;

#[repr(C)]
pub struct PsFrame {
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub frames_lost: i32,
    pub data: [*const u8; 3],
    pub linesize: [i32; 3],
    pub opaque: *mut c_void,
}

#[repr(C)]
pub struct PsDiscovery {
    _p: [u8; 0],
}
#[repr(C)]
pub struct PsRegist {
    _p: [u8; 0],
}
#[repr(C)]
pub struct PsSession {
    _p: [u8; 0],
}

#[link(name = "psnative")]
unsafe extern "C" {
    pub fn ps_init(level_mask: u32, cb: Option<LogCb>, user: *mut c_void) -> c_int;

    pub fn ps_discovery_start(cb: DiscoveryCb, user: *mut c_void) -> *mut PsDiscovery;
    pub fn ps_discovery_stop(d: *mut PsDiscovery);
    pub fn ps_wakeup(host: *const c_char, regist_key: *const c_char, ps5: c_int) -> c_int;

    pub fn ps_regist_start(
        host: *const c_char,
        ps5: c_int,
        broadcast: c_int,
        psn_account_id: *const u8,
        pin: u32,
        cb: RegistCb,
        user: *mut c_void,
    ) -> *mut PsRegist;
    pub fn ps_regist_free(r: *mut PsRegist);

    pub fn ps_session_start(
        config: *const PsSessionConfig,
        cbs: *const PsSessionCallbacks,
        user: *mut c_void,
    ) -> *mut PsSession;
    pub fn ps_session_stop(s: *mut PsSession);
    pub fn ps_session_free(s: *mut PsSession);
    pub fn ps_session_set_login_pin(s: *mut PsSession, pin: *const c_char) -> c_int;
    pub fn ps_session_go_home(s: *mut PsSession) -> c_int;
    pub fn ps_session_goto_bed(s: *mut PsSession) -> c_int;
    pub fn ps_session_set_controller(s: *mut PsSession, state: *const PsController) -> c_int;
    pub fn ps_session_pull_frame(s: *mut PsSession, out: *mut PsFrame) -> c_int;
    pub fn ps_frame_release(frame: *mut PsFrame);
}

/// Reads a fixed-size, NUL-terminated C char array into a String.
pub fn c_array_to_string(arr: &[c_char]) -> String {
    let bytes: Vec<u8> = arr
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
