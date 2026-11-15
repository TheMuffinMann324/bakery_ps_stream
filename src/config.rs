//! Persistent settings and registered consoles, stored as JSON in %APPDATA%\ps-stream.

use crate::remote::RegisteredHost;
use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const PROTECTED_HEADER: &[u8] = b"PS Stream DPAPI v1\n";

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedHost {
    pub nickname: String,
    pub mac: String,
    pub ps5: bool,
    /// Last known IP address, used when the console isn't discovered.
    pub last_addr: String,
    pub regist_key: String,
    pub rp_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<[u8; 8]>,
}

impl SavedHost {
    pub fn from_registered(h: &RegisteredHost, addr: &str, account_id: [u8; 8]) -> Self {
        Self {
            nickname: h.nickname.clone(),
            mac: h.mac_hex(),
            ps5: h.ps5,
            last_addr: addr.to_owned(),
            regist_key: B64.encode(h.regist_key),
            rp_key: B64.encode(h.rp_key),
            account_id: Some(account_id),
        }
    }

    pub fn to_registered(&self) -> Option<RegisteredHost> {
        let regist_key: [u8; 16] = B64.decode(&self.regist_key).ok()?.try_into().ok()?;
        let rp_key: [u8; 16] = B64.decode(&self.rp_key).ok()?.try_into().ok()?;
        let mut mac = [0u8; 6];
        for (i, b) in mac.iter_mut().enumerate() {
            *b = u8::from_str_radix(self.mac.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(RegisteredHost {
            ps5: self.ps5,
            nickname: self.nickname.clone(),
            mac,
            regist_key,
            rp_key,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// PSN account ID, base64 of 8 bytes.
    pub psn_account_id: String,
    pub resolution: i32,
    pub fps: i32,
    pub codec: i32,
    pub bitrate_kbps: u32,
    pub hw_decode: bool,
    pub keyboard_controls: bool,
    pub fullscreen_on_connect: bool,
    pub streamer_mode: bool,
    pub volume_percent: u8,
    /// Watch only disables app controls and prefers a second account when linked.
    pub watch_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            psn_account_id: String::new(),
            resolution: 4, // 1080p
            fps: 60,
            codec: 1, // H.265
            bitrate_kbps: 0,
            hw_decode: true,
            keyboard_controls: true,
            fullscreen_on_connect: false,
            streamer_mode: false,
            volume_percent: 100,
            watch_only: false,
        }
    }
}

impl Settings {
    pub fn account_id_bytes(&self) -> Result<[u8; 8], String> {
        decode_account_id(&self.psn_account_id)
    }
}

pub fn decode_account_id(input: &str) -> Result<[u8; 8], String> {
    let bytes = B64
        .decode(input.trim())
        .map_err(|_| "The PSN account ID is not valid base64".to_owned())?;
    bytes
        .try_into()
        .map_err(|_| "The PSN account ID must decode to exactly 8 bytes".to_owned())
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    pub hosts: Vec<SavedHost>,
    pub watch_hosts: Vec<SavedHost>,
    #[serde(skip)]
    read_only: bool,
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ps-stream").map(|d| d.config_dir().join("config.json"))
}

impl Config {
    pub fn load() -> (Self, Option<String>) {
        let Some(path) = path() else {
            return (Self::default(), None);
        };
        match Self::load_from(&path) {
            Ok((config, legacy)) => {
                let warning = legacy
                    .then(|| config.save_to(&path))
                    .and_then(Result::err)
                    .map(|err| format!("Could not protect existing settings: {err:#}"));
                (config, warning)
            }
            Err(err) => (
                Self {
                    read_only: true,
                    ..Self::default()
                },
                Some(format!(
                    "Could not read settings. The existing file is preserved: {err:#}"
                )),
            ),
        }
    }

    fn load_from(path: &Path) -> anyhow::Result<(Self, bool)> {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok((Self::default(), false));
            }
            Err(err) => return Err(err.into()),
        };
        if let Some(blob) = data.strip_prefix(PROTECTED_HEADER) {
            let json = crate::dpapi::unprotect(blob)
                .context("Windows could not decrypt the settings file")?;
            return Ok((serde_json::from_slice(&json)?, false));
        }
        Ok((serde_json::from_slice(&data)?, true))
    }

    pub fn save(&self) {
        if self.read_only {
            return;
        }
        if let Some(path) = path()
            && let Err(err) = self.save_to(&path)
        {
            crate::remote::log_line(format!("[E] Could not save settings: {err:#}"));
        }
    }

    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_vec(self)?;
        let encrypted =
            crate::dpapi::protect(&json).context("Windows could not encrypt the settings")?;
        let dir = path.parent().context("Missing settings directory")?;
        std::fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(PROTECTED_HEADER)?;
        file.write_all(&encrypted)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|err| err.error)?;
        Ok(())
    }

    pub fn validate_watch_account(
        &self,
        account: [u8; 8],
        play_host: Option<&SavedHost>,
    ) -> Result<(), String> {
        if self.settings.account_id_bytes().ok() == Some(account)
            || play_host.and_then(|host| host.account_id) == Some(account)
        {
            return Err("Use a second PSN account for Watch only. Your main account is for the controller on the console.".into());
        }
        Ok(())
    }

    pub fn hosts_mut(&mut self, watch_only: bool) -> &mut Vec<SavedHost> {
        if watch_only {
            &mut self.watch_hosts
        } else {
            &mut self.hosts
        }
    }

    pub fn upsert_host(&mut self, host: SavedHost, watch_only: bool) {
        let hosts = self.hosts_mut(watch_only);
        if let Some(existing) = hosts.iter_mut().find(|h| h.mac == host.mac) {
            *existing = host;
        } else {
            hosts.push(host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_play_link_survives_watch_link_registration() {
        let mut config: Config = serde_json::from_str(
            r#"{
            "settings": {"psn_account_id": "AQEBAQEBAQE=", "watch_only": true},
            "hosts": [{
                "nickname": "PS5", "mac": "010203040506", "ps5": true,
                "last_addr": "192.0.2.1", "regist_key": "play-key", "rp_key": "play-rp-key"
            }]
        }"#,
        )
        .unwrap();

        assert!(config.watch_hosts.is_empty());
        assert!(!config.settings.streamer_mode);
        assert_eq!(config.settings.volume_percent, 100);
        assert_eq!(config.hosts[0].account_id, None);
        assert!(
            config
                .validate_watch_account([1; 8], config.hosts.first())
                .is_err()
        );

        let mut watch = config.hosts[0].clone();
        watch.account_id = Some([2; 8]);
        watch.regist_key = "watch-key".into();
        assert!(
            config
                .validate_watch_account([2; 8], config.hosts.first())
                .is_ok()
        );
        config.upsert_host(watch, true);
        config.settings.streamer_mode = true;
        config.settings.volume_percent = 0;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let (legacy_config, legacy) = Config::load_from(&path).unwrap();
        assert!(legacy);

        legacy_config.save_to(&path).unwrap();
        let contents = std::fs::read(&path).unwrap();
        assert!(contents.starts_with(PROTECTED_HEADER));
        assert!(
            !contents
                .windows(b"watch-key".len())
                .any(|bytes| bytes == b"watch-key")
        );
        let (reloaded, legacy) = Config::load_from(&path).unwrap();
        assert!(!legacy);
        assert!(reloaded.settings.streamer_mode);
        assert_eq!(reloaded.settings.volume_percent, 0);
        assert_eq!(reloaded.hosts[0].regist_key, "play-key");
        assert_eq!(reloaded.watch_hosts[0].regist_key, "watch-key");
        assert_eq!(reloaded.watch_hosts[0].account_id, Some([2; 8]));
    }
}
