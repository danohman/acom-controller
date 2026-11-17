//! Persistent user settings, stored as JSON in the per-user config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Connection {
    #[default]
    Serial,
    /// Connect to another instance that shares its amplifier over TCP.
    Remote,
    /// Built-in amplifier simulator.
    Demo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RadioSource {
    #[default]
    Off,
    /// N1MM+ style UDP "RadioInfo" XML broadcasts.
    N1mmUdp,
    /// Hamlib rigctld TCP server.
    Rigctld,
    /// Shared-memory frequency export used by some loggers.
    SharedMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PowerPulse {
    #[default]
    Dtr,
    Rts,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub connection: Connection,
    pub com_port: String,
    pub remote_host: String,
    pub remote_port: u16,
    pub auto_connect: bool,
    pub auto_reconnect: bool,

    pub share_enabled: bool,
    pub share_port: u16,

    pub radio_source: RadioSource,
    pub radio_number: u8,
    pub n1mm_port: u16,
    pub rigctld_host: String,
    pub rigctld_port: u16,
    /// Automatically switch the amplifier band to follow the radio.
    pub follow_radio: bool,

    pub power_on_enabled: bool,
    pub power_pulse: PowerPulse,
    pub power_pulse_ms: u32,
    pub power_hold: bool,

    pub fahrenheit: bool,
    pub always_on_top: bool,
    pub dark_mode: bool,
    pub peak_hold: bool,
    pub swr_alert: f32,
    pub confirm_dangerous: bool,
    pub ui_scale: f32,
    /// Slim single-column layout.
    pub compact: bool,
    /// Last outer window positions (points): compact main window, full main window, settings window.
    pub pos_compact: Option<[f32; 2]>,
    pub pos_full: Option<[f32; 2]>,
    pub pos_settings: Option<[f32; 2]>,

    pub sound_enabled: bool,
    pub sound_volume: f32,
    pub sound_faults: bool,
    pub sound_swr: bool,
    pub sound_link: bool,

    /// Look for a newer GitHub release at startup.
    pub check_updates: bool,
    pub tray_enabled: bool,
    /// Closing the window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connection: Connection::Demo,
            com_port: String::new(),
            remote_host: "127.0.0.1".into(),
            remote_port: 7878,
            auto_connect: true,
            auto_reconnect: true,
            share_enabled: false,
            share_port: 7878,
            radio_source: RadioSource::Off,
            radio_number: 1,
            n1mm_port: 12060,
            rigctld_host: "127.0.0.1".into(),
            rigctld_port: 4532,
            follow_radio: true,
            power_on_enabled: false,
            power_pulse: PowerPulse::Both,
            power_pulse_ms: 1000,
            power_hold: false,
            fahrenheit: false,
            always_on_top: false,
            dark_mode: true,
            peak_hold: true,
            swr_alert: 2.5,
            confirm_dangerous: true,
            ui_scale: 1.0,
            compact: false,
            pos_compact: None,
            pos_full: None,
            pos_settings: None,
            sound_enabled: true,
            sound_volume: 0.5,
            sound_faults: true,
            sound_swr: true,
            sound_link: true,
            check_updates: true,
            tray_enabled: true,
            close_to_tray: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "acom-controller").map(|d| d.config_dir().join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }
}
