//! Safe wrappers over the native bridge: logging, discovery, registration and sessions.

use crate::audio::AudioOutput;
use crate::ffi;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

// ------------------------------------------------------------------- logging

const LOG_LINES: usize = 500;

static LOG: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn log_buffer() -> &'static Mutex<VecDeque<String>> {
    LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_LINES)))
}

pub fn log_line(line: String) {
    eprintln!("{line}");
    let mut buf = log_buffer().lock();
    if buf.len() == LOG_LINES {
        buf.pop_front();
    }
    buf.push_back(line);
}

pub fn log_snapshot() -> Vec<String> {
    log_buffer().lock().iter().cloned().collect()
}

unsafe extern "C" fn log_cb(level: c_int, msg: *const c_char, _user: *mut c_void) {
    let msg = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    let tag = match level {
        1 => 'E',
        2 => 'W',
        4 => 'I',
        8 => 'V',
        _ => 'D',
    };
    log_line(format!("[{tag}] {}", msg.trim_end()));
}

/// Initialise libchiaki. Call once at startup.
pub fn init() -> anyhow::Result<()> {
    // error | warning | info
    let err = unsafe { ffi::ps_init(1 | 2 | 4, Some(log_cb), std::ptr::null_mut()) };
    anyhow::ensure!(err == 0, "chiaki_lib_init failed with error {err}");
    Ok(())
}

// ----------------------------------------------------------------- discovery

#[derive(Clone, Debug)]
pub struct DiscoveredHost {
    pub awake: bool,
    pub standby: bool,
    pub ps5: bool,
    pub addr: String,
    pub name: String,
    /// Console MAC as uppercase hex, matches [`RegisteredHost::mac_hex`].
    pub host_id: String,
    pub running_app: String,
}

struct DiscoveryShared {
    hosts: Mutex<Vec<DiscoveredHost>>,
    ctx: eframe::egui::Context,
}

pub struct Discovery {
    raw: *mut ffi::PsDiscovery,
    shared: Arc<DiscoveryShared>,
}

unsafe extern "C" fn discovery_cb(hosts: *const ffi::PsHost, count: usize, user: *mut c_void) {
    let shared = unsafe { &*(user as *const DiscoveryShared) };
    let hosts = if count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(hosts, count) }
    };
    let list = hosts
        .iter()
        .map(|h| DiscoveredHost {
            awake: h.state == 1,
            standby: h.state == 2,
            ps5: h.is_ps5 != 0,
            addr: ffi::c_array_to_string(&h.host_addr),
            name: ffi::c_array_to_string(&h.host_name),
            host_id: ffi::c_array_to_string(&h.host_id).to_uppercase(),
            running_app: ffi::c_array_to_string(&h.running_app_name),
        })
        .collect();
    *shared.hosts.lock() = list;
    shared.ctx.request_repaint();
}

impl Discovery {
    pub fn start(ctx: eframe::egui::Context) -> anyhow::Result<Self> {
        let shared = Arc::new(DiscoveryShared {
            hosts: Mutex::new(Vec::new()),
            ctx,
        });
        let raw =
            unsafe { ffi::ps_discovery_start(discovery_cb, Arc::as_ptr(&shared) as *mut c_void) };
        anyhow::ensure!(!raw.is_null(), "could not start console discovery");
        Ok(Self { raw, shared })
    }

    pub fn hosts(&self) -> Vec<DiscoveredHost> {
        self.shared.hosts.lock().clone()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        // Joins the discovery thread, so `shared` outlives every callback.
        unsafe { ffi::ps_discovery_stop(self.raw) };
    }
}

pub fn wakeup(addr: &str, regist_key: &[u8; 16], ps5: bool) -> anyhow::Result<()> {
    let host = CString::new(addr)?;
    let err = unsafe {
        ffi::ps_wakeup(
            host.as_ptr(),
            regist_key.as_ptr() as *const c_char,
            ps5 as c_int,
        )
    };
    anyhow::ensure!(err == 0, "wakeup failed (error {err})");
    Ok(())
}

// -------------------------------------------------------------- registration

#[derive(Clone, Debug)]
pub struct RegisteredHost {
    pub ps5: bool,
    pub nickname: String,
    pub mac: [u8; 6],
    pub regist_key: [u8; 16],
    pub rp_key: [u8; 16],
}

impl RegisteredHost {
    pub fn mac_hex(&self) -> String {
        self.mac.iter().map(|b| format!("{b:02X}")).collect()
    }
}

pub enum RegistState {
    Running,
    Done(Result<RegisteredHost, String>),
}

struct RegistShared {
    state: Mutex<RegistState>,
    ctx: eframe::egui::Context,
}

pub struct Registration {
    raw: *mut ffi::PsRegist,
    shared: Arc<RegistShared>,
}

unsafe extern "C" fn regist_cb(
    result: c_int,
    host: *const ffi::PsRegisteredHost,
    user: *mut c_void,
) {
    let shared = unsafe { &*(user as *const RegistShared) };
    let outcome = match result {
        0 if !host.is_null() => {
            let h = unsafe { &*host };
            let mut regist_key = [0u8; 16];
            for (d, s) in regist_key.iter_mut().zip(h.rp_regist_key.iter()) {
                *d = *s as u8;
            }
            Ok(RegisteredHost {
                ps5: h.target >= 1_000_000,
                nickname: ffi::c_array_to_string(&h.server_nickname),
                mac: h.server_mac,
                regist_key,
                rp_key: h.rp_key,
            })
        }
        2 => Err("Registration was canceled".to_owned()),
        _ => Err("Registration failed. Check the PIN, the account ID and that Remote Play is enabled on the console.".to_owned()),
    };
    *shared.state.lock() = RegistState::Done(outcome);
    shared.ctx.request_repaint();
}

impl Registration {
    pub fn start(
        ctx: eframe::egui::Context,
        addr: &str,
        ps5: bool,
        account_id: &[u8; 8],
        pin: u32,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new(RegistShared {
            state: Mutex::new(RegistState::Running),
            ctx,
        });
        let host = CString::new(addr)?;
        let raw = unsafe {
            ffi::ps_regist_start(
                host.as_ptr(),
                ps5 as c_int,
                0,
                account_id.as_ptr(),
                pin,
                regist_cb,
                Arc::as_ptr(&shared) as *mut c_void,
            )
        };
        anyhow::ensure!(!raw.is_null(), "could not start registration");
        Ok(Self { raw, shared })
    }

    /// Returns the result once, after the console has answered.
    pub fn take_result(&self) -> Option<Result<RegisteredHost, String>> {
        let mut state = self.shared.state.lock();
        match std::mem::replace(&mut *state, RegistState::Running) {
            RegistState::Running => None,
            RegistState::Done(r) => Some(r),
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe { ffi::ps_regist_free(self.raw) };
    }
}

// ------------------------------------------------------------------- session

#[derive(Clone, Debug, PartialEq)]
pub enum SessionStatus {
    Connecting,
    Connected,
    /// Console asked for its login passcode; `true` if the last one was wrong.
    NeedsLoginPin(bool),
    Ended(String),
}

pub struct StreamOptions {
    pub resolution: i32,
    pub fps: i32,
    pub codec: i32,
    pub bitrate_kbps: u32,
    pub hw_decode: bool,
    pub volume_percent: u8,
}

struct SessionShared {
    status: Mutex<SessionStatus>,
    nickname: Mutex<Option<String>>,
    audio: Mutex<Option<AudioOutput>>,
    volume: Arc<AtomicU8>,
    ctx: eframe::egui::Context,
}

pub struct Session {
    raw: *mut ffi::PsSession,
    shared: Arc<SessionShared>,
    last_controller: Option<ffi::PsController>,
}

unsafe extern "C" fn session_event_cb(
    ty: c_int,
    a: c_int,
    _b: c_int,
    s: *const c_char,
    user: *mut c_void,
) {
    let shared = unsafe { &*(user as *const SessionShared) };
    let text = if s.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
    };
    match ty {
        ffi::EVENT_CONNECTED => *shared.status.lock() = SessionStatus::Connected,
        ffi::EVENT_QUIT => *shared.status.lock() = SessionStatus::Ended(text),
        ffi::EVENT_LOGIN_PIN_REQUEST => {
            *shared.status.lock() = SessionStatus::NeedsLoginPin(a != 0)
        }
        ffi::EVENT_NICKNAME => *shared.nickname.lock() = Some(text),
        _ => {}
    }
    shared.ctx.request_repaint();
}

unsafe extern "C" fn audio_settings_cb(channels: u32, rate: u32, user: *mut c_void) {
    let shared = unsafe { &*(user as *const SessionShared) };
    log_line(format!("[I] Audio stream: {channels} ch @ {rate} Hz"));
    let out = match AudioOutput::new(channels as u16, rate, Arc::clone(&shared.volume)) {
        Ok(out) => Some(out),
        Err(e) => {
            log_line(format!("[E] Audio output unavailable: {e:#}"));
            None
        }
    };
    *shared.audio.lock() = out;
}

unsafe extern "C" fn audio_frame_cb(samples: *const i16, frames: usize, user: *mut c_void) {
    let shared = unsafe { &*(user as *const SessionShared) };
    if let Some(out) = shared.audio.lock().as_mut() {
        let len = frames * out.source_channels() as usize;
        out.push(unsafe { std::slice::from_raw_parts(samples, len) });
    }
}

unsafe extern "C" fn frame_available_cb(user: *mut c_void) {
    let shared = unsafe { &*(user as *const SessionShared) };
    shared.ctx.request_repaint();
}

impl Session {
    pub fn start(
        ctx: eframe::egui::Context,
        addr: &str,
        host: &RegisteredHost,
        account_id: &[u8; 8],
        opts: &StreamOptions,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new(SessionShared {
            status: Mutex::new(SessionStatus::Connecting),
            nickname: Mutex::new(None),
            audio: Mutex::new(None),
            volume: Arc::new(AtomicU8::new(opts.volume_percent)),
            ctx,
        });
        let host_c = CString::new(addr)?;
        let mut regist_key = [0 as c_char; 16];
        for (d, s) in regist_key.iter_mut().zip(host.regist_key.iter()) {
            *d = *s as c_char;
        }
        let config = ffi::PsSessionConfig {
            host: host_c.as_ptr(),
            ps5: host.ps5 as i32,
            regist_key,
            morning: host.rp_key,
            psn_account_id: *account_id,
            resolution: opts.resolution,
            fps: opts.fps,
            codec: opts.codec,
            bitrate: opts.bitrate_kbps,
            hw_decode: opts.hw_decode as i32,
        };
        let cbs = ffi::PsSessionCallbacks {
            event: Some(session_event_cb),
            audio_settings: Some(audio_settings_cb),
            audio_frame: Some(audio_frame_cb),
            frame_available: Some(frame_available_cb),
        };
        let raw =
            unsafe { ffi::ps_session_start(&config, &cbs, Arc::as_ptr(&shared) as *mut c_void) };
        anyhow::ensure!(
            !raw.is_null(),
            "could not start the session, see the log for details"
        );
        Ok(Self {
            raw,
            shared,
            last_controller: None,
        })
    }

    pub fn status(&self) -> SessionStatus {
        self.shared.status.lock().clone()
    }

    pub fn set_volume(&self, percent: u8) {
        self.shared.volume.store(percent, Ordering::Relaxed);
    }

    pub fn nickname(&self) -> Option<String> {
        self.shared.nickname.lock().clone()
    }

    pub fn set_login_pin(&self, pin: &str) {
        if let Ok(pin) = CString::new(pin) {
            *self.shared.status.lock() = SessionStatus::Connecting;
            unsafe { ffi::ps_session_set_login_pin(self.raw, pin.as_ptr()) };
        }
    }

    pub fn go_home(&self) {
        unsafe { ffi::ps_session_go_home(self.raw) };
    }

    pub fn goto_bed(&self) {
        unsafe { ffi::ps_session_goto_bed(self.raw) };
    }

    /// Sends controller state, skipping it when nothing changed.
    pub fn set_controller(&mut self, state: ffi::PsController) {
        if self.last_controller == Some(state) {
            return;
        }
        self.last_controller = Some(state);
        unsafe { ffi::ps_session_set_controller(self.raw, &state) };
    }

    /// Newest decoded frame, if one arrived since the last call.
    pub fn pull_frame(&self) -> Option<VideoFrame> {
        let mut frame = ffi::PsFrame {
            width: 0,
            height: 0,
            format: 0,
            frames_lost: 0,
            data: [std::ptr::null(); 3],
            linesize: [0; 3],
            opaque: std::ptr::null_mut(),
        };
        let got = unsafe { ffi::ps_session_pull_frame(self.raw, &mut frame) };
        (got != 0).then_some(VideoFrame { raw: frame })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Stops and joins every chiaki thread before `shared` can be freed.
        unsafe { ffi::ps_session_free(self.raw) };
    }
}

/// A decoded frame in system memory. Plane data is valid until drop.
pub struct VideoFrame {
    raw: ffi::PsFrame,
}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        self.raw.width as u32
    }
    pub fn height(&self) -> u32 {
        self.raw.height as u32
    }
    pub fn format(&self) -> i32 {
        self.raw.format
    }
    /// Plane bytes and stride. `rows` is the number of rows in that plane.
    pub fn plane(&self, index: usize, rows: u32) -> (&[u8], u32) {
        let stride = self.raw.linesize[index];
        let ptr = self.raw.data[index];
        if ptr.is_null() || stride <= 0 {
            return (&[], 0);
        }
        let len = stride as usize * rows as usize;
        (
            unsafe { std::slice::from_raw_parts(ptr, len) },
            stride as u32,
        )
    }
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        unsafe { ffi::ps_frame_release(&mut self.raw) };
    }
}
