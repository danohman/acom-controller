//! Main application window.

use crate::config::{CompactGraph, Config, Connection, PowerPulse, RadioSource};
use crate::link::{self, Cmd, Event, LinkHandle};
use crate::protocol::bands::{BANDS, band_by_number, band_for_frequency};
use crate::protocol::commands::{self, ModeRequest};
use crate::protocol::errors::{self, Severity};
use crate::protocol::messages::{AmpMode, BAUD_TABLE, addr};
use crate::radio::{self, RadioEvent, RadioHandle};
use crate::state::AmpState;
use crate::update::UpdateStatus;
use crate::widgets::{self, AMBER, BLUE, GREEN, MUTED, Meter, RED, Zones};
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot, PlotPoints};
mod amp_windows;

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum LinkStatus {
    Idle,
    Connecting,
    Connected(String),
    Failed(String),
}

const BMC_URL: &str = "https://buymeacoffee.com/wd4dan";
const PAYPAL_URL: &str = "https://www.paypal.com/donate/?hosted_button_id=ZUP4M98EWMRD4";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    Connection,
    PowerSharing,
    Radio,
    Display,
    Alerts,
    System,
    About,
}

impl SettingsTab {
    const ALL: [SettingsTab; 7] = [
        Self::Connection,
        Self::PowerSharing,
        Self::Radio,
        Self::Display,
        Self::Alerts,
        Self::System,
        Self::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::PowerSharing => "Power & sharing",
            Self::Radio => "Radio",
            Self::Display => "Display",
            Self::Alerts => "Alerts",
            Self::System => "System",
            Self::About => "About",
        }
    }
}

/// What the settings window asks the app to do after a frame.
#[derive(Default)]
struct SettingsFlags {
    changed: bool,
    reconnect: bool,
    restart_radio: bool,
    relayout: bool,
    moved: bool,
}

pub struct App {
    cfg: Config,
    link: Option<LinkHandle>,
    status: LinkStatus,
    user_disconnected: bool,
    next_retry: Option<Instant>,
    amp: AmpState,
    radio: Option<RadioHandle>,
    radio_status: String,
    radio_khz: Option<u32>,
    last_follow: Option<Instant>,
    clients: usize,
    ports: Vec<String>,
    ports_checked: Option<Instant>,
    show_settings: bool,
    settings_tab: SettingsTab,
    update: UpdateStatus,
    update_rx: Option<std::sync::mpsc::Receiver<UpdateStatus>>,
    update_silent: bool,
    install_rx: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    update_error: Option<String>,
    show_log: bool,
    /// Waiting for the user to confirm powering the amplifier off.
    confirm_power_off: bool,
    comm_log: VecDeque<String>,
    toast: Option<(String, Instant)>,
    plot_metric: usize,
    settings_dirty: bool,
    last_info_req: Option<Instant>,
    logo: egui::TextureHandle,
    bmc_logo: egui::TextureHandle,
    paypal_logo: egui::TextureHandle,
    sound: crate::sound::Sound,
    tray: Option<crate::tray::Tray>,
    // alert edge detection
    was_alive: bool,
    link_lost: bool,
    seen_alarms: usize,
    last_swr_alert: Option<Instant>,
    // hard-fault viewer / amplifier settings editor
    show_faults: bool,
    hf_next: Option<u8>,
    hf_last: Option<Instant>,
    hf_step: u8,
    hf_base: (u32, u32),
    show_amp_settings: bool,
    edit: Option<amp_windows::AmpEdit>,
    /// Window-position memory: frames to ignore after a programmatic move, startup flag, last change.
    pos_skip: u8,
    /// Where the settings window was opened; fixed while open so it is not re-placed every frame.
    settings_start: Option<egui::Pos2>,
    /// Compact layout's "choose graph" popup: (slot, screen position, frames open, has had focus).
    graph_menu: Option<(usize, egui::Pos2, u32, bool)>,
    pos_init: bool,
    pos_changed: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::update::cleanup_old();
        let cfg = Config::load();
        crate::theme::apply(&cc.egui_ctx, cfg.dark_mode, cfg.ui_scale);
        let mut app = Self {
            cfg,
            link: None,
            status: LinkStatus::Idle,
            user_disconnected: false,
            next_retry: None,
            amp: AmpState::default(),
            radio: None,
            radio_status: String::new(),
            radio_khz: None,
            last_follow: None,
            clients: 0,
            ports: Vec::new(),
            ports_checked: None,
            show_settings: false,
            settings_tab: SettingsTab::default(),
            update: UpdateStatus::Idle,
            update_rx: None,
            update_silent: false,
            install_rx: None,
            update_error: None,
            show_log: false,
            confirm_power_off: false,
            comm_log: VecDeque::new(),
            toast: None,
            plot_metric: 0,
            settings_dirty: false,
            last_info_req: None,
            logo: crate::icon::texture(&cc.egui_ctx),
            bmc_logo: crate::icon::image_texture(&cc.egui_ctx, "bmc", include_bytes!("../assets/bmc-button.png")),
            paypal_logo: crate::icon::image_texture(&cc.egui_ctx, "paypal", include_bytes!("../assets/paypal-logo.png")),
            sound: crate::sound::Sound::new(),
            tray: None,
            was_alive: false,
            link_lost: false,
            seen_alarms: 0,
            last_swr_alert: None,
            show_faults: false,
            hf_next: None,
            hf_last: None,
            hf_step: 0,
            hf_base: (0, 0),
            pos_skip: 0,
            settings_start: None,
            graph_menu: None,
            pos_init: false,
            pos_changed: None,
            show_amp_settings: false,
            edit: None,
        };
        app.sync_tray(&cc.egui_ctx);
        app.refresh_ports();
        if app.cfg.auto_connect {
            app.connect(&cc.egui_ctx);
        }
        app.restart_radio(&cc.egui_ctx);
        if app.cfg.check_updates {
            app.update_silent = true;
            app.start_update_check(&cc.egui_ctx);
        }
        app
    }

    fn refresh_ports(&mut self) {
        self.ports = serialport::available_ports()
            .map(|v| v.into_iter().map(|p| p.port_name).collect())
            .unwrap_or_default();
        self.ports.sort();
        self.ports_checked = Some(Instant::now());
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.disconnect(false);
        self.user_disconnected = false;
        self.amp.reset_link();
        self.last_info_req = None;
        self.status = LinkStatus::Connecting;
        self.link = Some(link::start(&self.cfg, ctx.clone()));
    }

    fn disconnect(&mut self, by_user: bool) {
        self.link = None;
        self.clients = 0;
        if by_user {
            self.user_disconnected = true;
            self.status = LinkStatus::Idle;
        }
    }

    fn restart_radio(&mut self, ctx: &egui::Context) {
        self.radio = None;
        self.radio_status.clear();
        self.radio_khz = None;
        self.radio = radio::start(&self.cfg, ctx.clone());
    }

    fn send(&mut self, wire: Vec<u8>) {
        if let Some(l) = &self.link {
            l.send(wire);
        }
    }

    fn start_update_check(&mut self, ctx: &egui::Context) {
        self.update = UpdateStatus::Checking;
        self.update_rx = Some(crate::update::spawn_check(ctx.clone()));
    }

    /// Persistent "new version available" notice with an Upgrade button, centred in `row`
    /// (preferably around `center_x`). Falls back to just the button when the row is narrow.
    fn update_banner(&mut self, ui: &mut egui::Ui, row: egui::Rect, center_x: f32) {
        let UpdateStatus::Available { version, url, asset } = self.update.clone() else { return };
        let installing = self.install_rx.is_some();
        let can_install = cfg!(windows) && asset.is_some();
        let failed = self.update_error.is_some();
        let msg = if failed { "Update failed".to_string() } else { format!("Version {version} available") };
        let btn_txt = if installing {
            "Downloading…"
        } else if failed {
            "Retry"
        } else if can_install {
            "Upgrade"
        } else {
            "Download"
        };
        let measure = |ui: &egui::Ui, t: &str, size: f32| {
            ui.painter().layout_no_wrap(t.to_owned(), egui::FontId::proportional(size), Color32::WHITE).size().x
        };
        let btn_w = measure(ui, btn_txt, 13.0) + 22.0 + if installing { 22.0 } else { 0.0 };
        let full = measure(ui, &msg, 12.5) + 8.0 + btn_w;
        let show_msg = full <= row.width();
        let w = if show_msg { full } else { btn_w };
        let left = (center_x - w / 2.0).clamp(row.left(), (row.right() - w).max(row.left()));
        let rect = egui::Rect::from_min_size(egui::pos2(left, row.top()), egui::vec2(w, row.height()));
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 8.0;
        if show_msg {
            let col = if failed { RED } else { child.visuals().strong_text_color() };
            let l = child.label(RichText::new(&msg).size(12.5).strong().color(col));
            if let Some(e) = &self.update_error {
                l.on_hover_text(e.as_str());
            }
        }
        if installing {
            child.add(egui::Spinner::new().size(14.0));
            child.label(RichText::new("Downloading…").size(12.5));
        } else {
            // Blue with white text: readable on both the light and dark themes.
            let btn = egui::Button::new(RichText::new(btn_txt).strong().size(13.0).color(Color32::WHITE))
                .fill(Color32::from_rgb(37, 99, 235));
            let tip = if can_install { "Download the new version and restart" } else { "Open the release page" };
            if child.add(btn).on_hover_text(tip).clicked() {
                if can_install {
                    self.start_install(ui.ctx());
                } else {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        }
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let UpdateStatus::Available { asset: Some(asset), .. } = &self.update else { return };
        let asset = asset.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::update::install(&asset));
            c.request_repaint();
        });
        self.install_rx = Some(rx);
        self.update_error = None;
    }

    fn poll_update(&mut self) {
        if let Some(rx) = &self.install_rx
            && let Ok(res) = rx.try_recv()
        {
            self.install_rx = None;
            match res {
                Ok(exe) => {
                    // New version is in place: save settings, start it and quit this copy.
                    self.cfg.save();
                    let _ = std::process::Command::new(&exe).spawn();
                    std::process::exit(0);
                }
                Err(e) => self.update_error = Some(e),
            }
        }
        let Some(rx) = &self.update_rx else { return };
        if let Ok(st) = rx.try_recv() {
            self.update_rx = None;
            if let UpdateStatus::Available { version, .. } = &st
                && self.update_silent
            {
                self.say(format!("Version {version} is available – see Settings › About"));
            }
            self.update_silent = false;
            self.update = st;
        }
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn log(&mut self, line: String) {
        self.comm_log.push_back(line);
        if self.comm_log.len() > 300 {
            self.comm_log.pop_front();
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(l) = &self.link {
            while let Ok(e) = l.rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                Event::Connected(name) => self.status = LinkStatus::Connected(name),
                Event::Disconnected(why) => {
                    self.link = None;
                    self.status = LinkStatus::Failed(why);
                    self.clients = 0;
                    if self.cfg.auto_reconnect && !self.user_disconnected {
                        self.next_retry = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
                Event::Message(m) => self.amp.apply(m),
                // The cyclic telemetry would drown everything else in the log.
                Event::Rx { address, len } if address != addr::TELEMETRY => self.log(format!("RX 0x{address:02X} ({len} B)")),
                Event::Rx { .. } => {}
                Event::Tx { len } => self.log(format!("TX ({len} B)")),
                Event::Clients(n) => self.clients = n,
                Event::Notice(t) => self.say(t),
            }
        }
        if let Some(t) = self.next_retry
            && Instant::now() >= t
            && self.link.is_none()
        {
            self.next_retry = None;
            self.connect(ctx);
        }
        self.request_missing_info();
        self.drive_fault_requests();
        self.check_alerts();
        let mut revents = Vec::new();
        if let Some(r) = &self.radio {
            while let Ok(e) = r.rx.try_recv() {
                revents.push(e);
            }
        }
        for e in revents {
            match e {
                RadioEvent::Status(s) => self.radio_status = s,
                RadioEvent::Frequency(k) => {
                    self.radio_khz = Some(k);
                    self.follow_radio(k);
                }
            }
        }
    }

    /// The amplifier only sends some messages on request (and band info only on change), and may
    /// ignore requests made before it is ready. Keep asking until everything has arrived.
    fn request_missing_info(&mut self) {
        if self.link.is_none() || !self.amp.alive() {
            return;
        }
        if self.last_info_req.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        let mut want = Vec::new();
        if self.amp.info.is_none() {
            want.push(addr::SYSTEM_INFO);
        }
        if self.amp.band.is_none() {
            want.push(addr::BAND_INFO);
        }
        if self.amp.settings.is_none() {
            want.push(addr::SETTINGS);
        }
        if want.is_empty() {
            return;
        }
        self.last_info_req = Some(Instant::now());
        for a in want {
            self.send(commands::request_message(a));
        }
    }

    /// Switch the amplifier band to match the radio, unless the amp is transmitting.
    fn follow_radio(&mut self, khz: u32) {
        if !self.cfg.follow_radio || self.link.is_none() || !self.amp.alive() {
            return;
        }
        let max = self.amp.model().max_band();
        let Some(target) = band_for_frequency(khz, max) else { return };
        let Some(cur) = self.amp.current_band() else { return };
        let tx = self.amp.telemetry.as_ref().is_some_and(|t| t.mode == AmpMode::OperateTx);
        let recently = self.last_follow.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
        if target.number != cur && !tx && !recently {
            self.last_follow = Some(Instant::now());
            self.send(commands::select_band(target.number));
        }
    }

    /// Create or drop the tray icon to match the setting.
    fn sync_tray(&mut self, ctx: &egui::Context) {
        if self.cfg.tray_enabled && self.tray.is_none() {
            self.tray = crate::tray::Tray::new(ctx);
        } else if !self.cfg.tray_enabled
            && let Some(t) = self.tray.take()
        {
            crate::tray::set_visible(ctx, &t.shared, true);
        }
    }

    fn play(&self, alert: crate::sound::Alert) {
        if self.cfg.sound_enabled {
            self.sound.play(alert, self.cfg.sound_volume);
        }
    }

    /// Detect events worth an audible alert.
    fn check_alerts(&mut self) {
        use crate::sound::Alert;
        let alive = self.amp.alive();
        if self.was_alive && !alive {
            self.link_lost = true;
            if self.cfg.sound_link {
                self.play(Alert::LinkLost);
            }
        } else if !self.was_alive && alive && self.link_lost {
            self.link_lost = false;
            if self.cfg.sound_link {
                self.play(Alert::LinkRestored);
            }
        }
        self.was_alive = alive;

        let n = self.amp.alarm_log.len();
        if n < self.seen_alarms {
            self.seen_alarms = n;
        }
        if n > self.seen_alarms {
            let worst = self.amp.alarm_log[self.seen_alarms..].iter().filter(|a| a.active).map(|a| a.severity).max();
            self.seen_alarms = n;
            if self.cfg.sound_faults {
                match worst {
                    Some(Severity::Warning) => self.play(Alert::Warning),
                    Some(Severity::SoftFault | Severity::HardFault) => self.play(Alert::Fault),
                    _ => {}
                }
            }
        }

        if self.cfg.sound_swr && alive {
            let high = self.amp.telemetry.as_ref().is_some_and(|t| t.forward_w > 5.0 && t.swr > self.cfg.swr_alert);
            if high && self.last_swr_alert.is_none_or(|t| t.elapsed() > Duration::from_secs(4)) {
                self.last_swr_alert = Some(Instant::now());
                self.play(Alert::Swr);
            }
        }
    }

    // ---------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let row_top = ui.cursor().top();
        let (mut left_end, mut right_start) = (0.0_f32, f32::MAX);
        ui.horizontal(|ui| {
            if !self.cfg.compact {
                ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(28.0, 28.0)));
                ui.label(RichText::new("ACOM").size(20.0).strong().color(BLUE));
                ui.label(RichText::new("Controller").size(20.0));
                ui.add_space(12.0);
            }
            match &self.status {
                LinkStatus::Idle => widgets::chip(ui, "Disconnected", MUTED),
                LinkStatus::Connecting => widgets::chip(ui, "Connecting…", AMBER),
                LinkStatus::Failed(_) => widgets::chip(ui, "Link error", RED),
                LinkStatus::Connected(n) => {
                    if self.amp.alive() {
                        widgets::chip(ui, &format!("● {n}"), GREEN)
                    } else {
                        widgets::chip(ui, &format!("{n} · no data"), AMBER)
                    }
                }
            }
            if let Some(t) = self.amp.telemetry.as_ref().filter(|_| !self.cfg.compact && self.amp.alive()) {
                let col = match t.mode {
                    AmpMode::OperateTx => RED,
                    AmpMode::OperateRx => GREEN,
                    AmpMode::Standby => BLUE,
                    _ => AMBER,
                };
                if t.mode == AmpMode::OperateTx {
                    widgets::tx_chip(ui, t.mode.label());
                } else {
                    widgets::chip(ui, t.mode.label(), col);
                }
            }
            if self.clients > 0 {
                widgets::chip(ui, &format!("{} remote client(s)", self.clients), BLUE);
            }
            left_end = ui.cursor().left();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = true;
                }
                if self.link.is_some() {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect(true);
                    }
                } else if ui.button("Connect").clicked() {
                    self.connect(ctx);
                }
                if self.link.is_some() && self.amp.alive() {
                    if ui.button(RichText::new("Power off").color(RED)).clicked() {
                        if self.cfg.confirm_dangerous {
                            self.confirm_power_off = true;
                        } else {
                            self.send(commands::set_mode(ModeRequest::TurnOff));
                        }
                    }
                } else if self.cfg.power_on_enabled
                    && self.cfg.connection == Connection::Serial
                    && ui.button("Power on").on_hover_text("Pulse the RS-232 control line configured in Settings").clicked()
                {
                    self.send_power_pulse(ctx);
                }
                right_start = ui.min_rect().left();
            });
        });
        if !self.cfg.compact {
            let row = egui::Rect::from_min_max(egui::pos2(left_end + 12.0, row_top + 2.0), egui::pos2(right_start - 12.0, row_top + 26.0));
            let center_x = ui.max_rect().center().x;
            self.update_banner(ui, row, center_x);
        }
        if let LinkStatus::Failed(why) = &self.status {
            ui.label(RichText::new(format!("⚠ {why}")).color(RED).size(12.5));
        }
    }

    fn send_power_pulse(&mut self, ctx: &egui::Context) {
        if self.link.is_none() {
            self.connect(ctx);
        }
        if let Some(l) = &self.link {
            let _ = l.tx.send(Cmd::PowerPulse(self.cfg.power_pulse, self.cfg.power_pulse_ms, self.cfg.power_hold));
            self.say("Power-on pulse sent");
        }
    }

    fn fmt_temp(&self, c: f32) -> String {
        if self.cfg.fahrenheit { format!("{:.0} °F", c * 9.0 / 5.0 + 32.0) } else { format!("{c:.0} °C") }
    }

    fn meters(&mut self, ui: &mut egui::Ui) {
        let model = self.amp.model();
        let Some(t) = self.amp.telemetry.clone().filter(|_| self.amp.alive()) else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(32.0));
                ui.add_space(8.0);
                ui.label(RichText::new("Waiting for amplifier…").size(18.0).color(MUTED));
            });
            return;
        };
        let (fwd_max, fwd_red) = model.forward_meter_w();
        let (refl_max, refl_red) = model.reflected_meter_w();
        let tx = t.mode == AmpMode::OperateTx;
        let carrier = t.forward_w > 0.0 || t.reflected_w > 0.0 || t.input_w > 0.0;
        let pk_on = self.cfg.peak_hold;
        let h = move |secs: f32| pk_on.then(|| std::time::Duration::from_secs_f32(secs));
        widgets::card(ui, "RF power", |ui| {
            ui.horizontal(|ui| {
                widgets::readout(ui, "FORWARD", &widgets::held_rf(ui, "fwd", tx, carrier, format!("{:.0} W", t.forward_w)), ui.visuals().strong_text_color());
                ui.add_space(18.0);
                widgets::readout(ui, "REFLECTED", &widgets::held_rf(ui, "refl", tx, carrier, format!("{:.0} W", t.reflected_w)), if t.reflected_w > 20.0 { AMBER } else { ui.visuals().strong_text_color() });
                ui.add_space(18.0);
                let swr_col = if !tx { ui.visuals().strong_text_color() } else if t.swr >= 3.0 { RED } else if t.swr >= self.cfg.swr_alert { AMBER } else { GREEN };
                widgets::readout(ui, "SWR", &widgets::held_rf(ui, "swr", tx, carrier, format!("{:.2}", t.swr.max(1.0))), swr_col);
                ui.add_space(18.0);
                widgets::readout(ui, "DRIVE", &widgets::held_rf(ui, "drive", tx, carrier, format!("{:.1} W", t.input_w)), ui.visuals().strong_text_color());
            });
            ui.add_space(6.0);
            Meter {
                label: "Forward power",
                value: t.forward_w,
                min: 0.0,
                max: fwd_max,
                text: widgets::rf_text(tx, carrier, format!("{:.0} W", t.forward_w)),
                zones: Zones::green_red(fwd_red),
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "Reflected power",
                value: t.reflected_w,
                min: 0.0,
                max: refl_max,
                text: widgets::rf_text(tx, carrier, format!("{:.0} W", t.reflected_w)),
                zones: Zones::green_red(refl_red),
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "SWR",
                value: t.swr.max(1.0),
                min: 1.0,
                max: 4.0,
                text: widgets::rf_text(tx, carrier, format!("{:.2}", t.swr.max(1.0))),
                zones: Zones { warn: self.cfg.swr_alert, crit: 3.0 },
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "Drive power",
                value: t.input_w,
                min: 0.0,
                max: 100.0,
                text: widgets::rf_text(tx, carrier, format!("{:.1} W", t.input_w)),
                zones: Zones { warn: 70.0, crit: 90.0 },
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
        });
        ui.add_space(8.0);
        widgets::card(ui, "Power amplifier", |ui| {
            let temp = t.temp1_c();
            Meter {
                label: "PA temperature",
                value: temp,
                min: 0.0,
                max: 100.0,
                text: self.fmt_temp(temp),
                zones: Zones::green_red(76.0),
                peak: h(60.0),
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "PA drain current",
                value: t.id1_a,
                min: 0.0,
                max: 60.0,
                text: format!("{:.1} A", t.id1_a),
                zones: Zones { warn: 45.0, crit: 55.0 },
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "PA voltage (HV)",
                value: t.hv1_v,
                min: 0.0,
                max: 60.0,
                text: format!("{:.1} V", t.hv1_v),
                zones: Zones::NONE,
                peak: h(2.5),
                height: 40.0,
            }
            .show(ui);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                widgets::readout(ui, "26 V RAIL", &format!("{:.1} V", t.vcc26_v), ui.visuals().strong_text_color());
                ui.add_space(14.0);
                widgets::readout(ui, "5 V RAIL", &format!("{:.2} V", t.vcc5_v), ui.visuals().strong_text_color());
                ui.add_space(14.0);
                widgets::readout(ui, "DISSIPATION", &format!("{:.0} W", t.dissipation_w()), ui.visuals().strong_text_color());
                ui.add_space(14.0);
                widgets::readout(ui, "FAN", &format!("{}/4", t.fan_speed), ui.visuals().strong_text_color());
                ui.add_space(14.0);
                widgets::readout(
                    ui,
                    "LPF",
                    &if t.lpf_channel == 0 { "–".into() } else { t.lpf_channel.to_string() },
                    if t.lpf_tuned() || t.lpf_channel == 0 { ui.visuals().strong_text_color() } else { AMBER },
                );
            });
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let connected = self.link.is_some() && self.amp.alive();
        let mode = self.amp.telemetry.as_ref().map(|t| t.mode).unwrap_or_default();
        let tx_allowed = self.amp.telemetry.as_ref().is_none_or(|t| t.tx_access_enabled());

        widgets::card(ui, "Operating mode", |ui| {
            ui.add_enabled_ui(connected, |ui| {
                ui.horizontal(|ui| {
                    let sz = egui::vec2(110.0, 38.0);
                    let sb = egui::Button::new(RichText::new("STANDBY").strong()).min_size(sz).selected(mode == AmpMode::Standby);
                    if ui.add(sb).clicked() {
                        self.send(commands::set_mode(ModeRequest::Standby));
                    }
                    let op = egui::Button::new(RichText::new("OPERATE").strong()).min_size(sz).selected(mode.is_operate());
                    if ui.add(op).clicked() {
                        self.send(commands::set_mode(ModeRequest::OperateRx));
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // Always labelled "Inhibit TX"; filled red while TX is inhibited.
                    let inhibit = if tx_allowed {
                        egui::Button::new("Inhibit TX")
                    } else {
                        egui::Button::new(RichText::new("Inhibit TX").strong().color(Color32::WHITE)).fill(RED)
                    };
                    let tip = if tx_allowed { "Prohibit entering OPERATE/TX" } else { "TX is inhibited – click to allow" };
                    if ui.add(inhibit).on_hover_text(tip).clicked() {
                        self.send(commands::set_mode(if tx_allowed { ModeRequest::ProhibitTx } else { ModeRequest::AllowTx }));
                    }
                    if ui.button("Clear faults").clicked() {
                        self.send(commands::clear_soft_faults());
                        self.send(commands::request_message(addr::ERROR_CODES));
                    }
                });
            });
        });
        ui.add_space(8.0);

        widgets::card(ui, "Band", |ui| {
            let max = self.amp.model().max_band();
            let cur_band = self.amp.current_band();
            ui.add_enabled_ui(connected, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for b in BANDS.iter().filter(|b| b.number <= max) {
                        let sel = Some(b.number) == cur_band;
                        let txt = RichText::new(b.name.replace(' ', "")).size(13.0);
                        let btn = if sel {
                            egui::Button::new(txt.strong().color(Color32::BLACK)).fill(GREEN)
                        } else {
                            egui::Button::new(txt)
                        }
                        .min_size(egui::vec2(52.0, 28.0));
                        if ui.add(btn).clicked() {
                            self.send(commands::select_band(b.number));
                        }
                    }
                });
            });
            if let Some(b) = &self.amp.band {
                ui.add_space(4.0);
                let name = self.amp.current_band().and_then(band_by_number).map(|x| x.name).unwrap_or("?");
                let line = format!("{} · {}–{} kHz", name, b.low_khz, b.high_khz);
                ui.label(RichText::new(line).size(12.5).color(MUTED));
            }
        });
        ui.add_space(8.0);

        widgets::card(ui, "Frequency", |ui| {
            let amp_khz = self.amp.telemetry.as_ref().map(|t| t.frequency_khz);
            ui.horizontal(|ui| {
                widgets::readout(
                    ui,
                    "AMPLIFIER SENSES",
                    &amp_khz.map(|k| format!("{:.3} MHz", k as f32 / 1000.0)).unwrap_or("–".into()),
                    ui.visuals().strong_text_color(),
                );
                if self.cfg.radio_source != RadioSource::Off {
                    ui.add_space(18.0);
                    widgets::readout(
                        ui,
                        "RADIO",
                        &self.radio_khz.map(|k| format!("{:.3} MHz", k as f32 / 1000.0)).unwrap_or("–".into()),
                        ui.visuals().strong_text_color(),
                    );
                }
            });
            if self.cfg.radio_source != RadioSource::Off {
                ui.label(RichText::new(&self.radio_status).size(12.0).color(MUTED));
                if ui.checkbox(&mut self.cfg.follow_radio, "Switch band automatically").changed() {
                    self.cfg.save();
                }
            }
        });
        ui.add_space(8.0);

        if let Some(sim) = self.link.as_ref().and_then(|l| l.sim.clone()) {
            widgets::card(ui, "Demo transmitter", |ui| {
                let mut tx = sim.transmitting.load(Ordering::Relaxed);
                if ui.checkbox(&mut tx, "Key transmitter").changed() {
                    sim.transmitting.store(tx, Ordering::Relaxed);
                }
                let mut drive = sim.drive_dw.load(Ordering::Relaxed) as f32 / 10.0;
                if ui.add(egui::Slider::new(&mut drive, 0.0..=60.0).text("Drive W")).changed() {
                    sim.drive_dw.store((drive * 10.0) as u32, Ordering::Relaxed);
                }
                let mut f = sim.fault.load(Ordering::Relaxed);
                if ui.checkbox(&mut f, "Inject high-SWR fault").changed() {
                    sim.fault.store(f, Ordering::Relaxed);
                }
            });
        }
    }

    fn alarms(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "Alarms", |ui| {
            let mut any = false;
            let active: Vec<u8> = {
                let mut v = self.amp.active_codes.clone();
                if let Some(t) = &self.amp.telemetry
                    && t.alarm_code != 0
                    && !v.contains(&t.alarm_code)
                {
                    v.push(t.alarm_code);
                }
                v
            };
            for code in active {
                if let Some(text) = errors::text(code) {
                    any = true;
                    let sev = errors::severity(code);
                    let col = sev_color(sev);
                    ui.horizontal_wrapped(|ui| {
                        widgets::chip(ui, sev.label(), col);
                        ui.label(text);
                    });
                }
            }
            if !any {
                ui.label(RichText::new("No active alarms").color(GREEN));
            }
            ui.add_space(6.0);
            ui.collapsing(format!("History ({})", self.amp.alarm_log.len()), |ui| {
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    for a in self.amp.alarm_log.iter().rev() {
                        let secs = a.when.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        let hms = format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60);
                        ui.label(RichText::new(format!("{hms} UTC  {}", a.text)).size(12.0).color(sev_color(a.severity)));
                    }
                });
            });
        });
    }

    fn history(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "History (last 5 min)", |ui| {
            ui.horizontal(|ui| {
                for (i, n) in ["Power", "SWR", "Temperature"].iter().enumerate() {
                    ui.selectable_value(&mut self.plot_metric, i, *n);
                }
                if ui.button("Clear").clicked() {
                    self.amp.reset_session_stats();
                }
                ui.label(
                    RichText::new(format!("max {:.0} W · max SWR {:.2}", self.amp.max_forward_w, self.amp.max_swr))
                        .size(12.0)
                        .color(MUTED),
                );
            });
            let now = Instant::now();
            let pts = |f: &dyn Fn(&crate::state::Sample) -> f32| -> PlotPoints<'static> {
                self.amp
                    .history
                    .iter()
                    .map(|s| [-(now.duration_since(s.t).as_secs_f64()), f(s) as f64])
                    .collect()
            };
            let fah = self.cfg.fahrenheit;
            let mut plot = Plot::new("history").height(170.0).allow_scroll(false).allow_zoom(false).allow_drag(false);
            match self.plot_metric {
                0 => plot = plot.include_y(0.0).include_y(100.0),
                1 => plot = plot.include_y(1.0).include_y(3.0),
                _ => plot = plot.include_y(20.0).include_y(50.0),
            }
            plot.show(ui, |p| match self.plot_metric {
                0 => {
                    p.line(Line::new("Forward W", pts(&|s| s.forward_w)).color(GREEN));
                    p.line(Line::new("Reflected W", pts(&|s| s.reflected_w)).color(RED));
                }
                1 => p.line(Line::new("SWR", pts(&|s| s.swr)).color(AMBER)),
                _ => p.line(Line::new("PA temp", pts(&move |s| if fah { s.temp_c * 9.0 / 5.0 + 32.0 } else { s.temp_c })).color(BLUE)),
            });
        });
    }

    fn info_card(&mut self, ui: &mut egui::Ui) {
        widgets::card(ui, "Amplifier", |ui| {
            if let Some(i) = &self.amp.info {
                ui.label(RichText::new(i.model.name()).strong().size(15.0));
                ui.label(format!("S/N {} · FW {} · CPU PCB {}", i.serial, i.firmware, i.cpu_pcb));
                let n = i.hard_fault_count;
                ui.horizontal(|ui| {
                    ui.label(format!("Hard faults recorded: {n}"));
                    if n > 0 && ui.small_button("View…").clicked() {
                        self.show_faults = true;
                    }
                });
            } else {
                ui.label(RichText::new("Not identified yet").color(MUTED));
            }
            let hello = format!("{}{}", self.amp.hello[0], self.amp.hello[1]).trim().to_string();
            if !hello.is_empty() {
                ui.label(format!("User text: {hello}"));
            }
            if let Some(s) = &self.amp.settings {
                let baud = BAUD_TABLE.iter().find(|(c, _)| *c == s.cat_baud).map(|x| x.1).unwrap_or("?");
                ui.label(RichText::new(format!("CAT baud {baud} · buzzer {}%", s.buzzer_volume)).size(12.0).color(MUTED));
            }
            if let Some(t) = &self.amp.telemetry {
                ui.label(RichText::new(format!("Online time: {}", fmt_online(t.clock_s as u64))).size(12.0).color(MUTED));
            }
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_log, "Show communication log");
                if ui.button("Amplifier settings…").clicked() {
                    self.show_amp_settings = true;
                    self.edit = None;
                }
            });
        });
    }

    /// Resize the window for the active layout.
    fn apply_layout(&mut self, ctx: &egui::Context) {
        let (size, min) = layout_sizes(self.cfg.compact);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min.into()));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
        let saved = if self.cfg.compact { self.cfg.pos_compact } else { self.cfg.pos_full };
        keep_on_screen(ctx, egui::vec2(size[0], size[1]), saved.map(|p| egui::pos2(p[0], p[1])));
        // Don't record the in-between positions while the window is being moved.
        self.pos_skip = 6;
        self.cfg.save();
    }

    /// Remember where the main window is, per layout; restore it on the first frame.
    fn track_main_pos(&mut self, ctx: &egui::Context) {
        if !self.pos_init {
            self.pos_init = true;
            let saved = if self.cfg.compact { self.cfg.pos_compact } else { self.cfg.pos_full };
            if let Some(p) = saved {
                let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())).unwrap_or(egui::vec2(480.0, 254.0));
                keep_on_screen(ctx, size, Some(egui::pos2(p[0], p[1])));
                self.pos_skip = 6;
            }
            return;
        }
        if self.pos_skip > 0 {
            self.pos_skip -= 1;
            return;
        }
        let Some(r) = ctx.input(|i| i.viewport().outer_rect) else { return };
        if r.min.x < -10000.0 || r.min.y < -10000.0 || ctx.input(|i| i.viewport().minimized == Some(true)) {
            return; // minimized / hidden windows report bogus positions
        }
        let cur = [r.min.x, r.min.y];
        let slot = if self.cfg.compact { &mut self.cfg.pos_compact } else { &mut self.cfg.pos_full };
        if *slot != Some(cur) {
            *slot = Some(cur);
            self.pos_changed = Some(Instant::now());
        }
        if self.pos_changed.is_some_and(|t| t.elapsed() > Duration::from_secs(2)) {
            self.pos_changed = None;
            self.cfg.save();
        }
    }

    /// Slim layout: everything an operator glances at, in one narrow column.
    fn compact_view(&mut self, ui: &mut egui::Ui) {
        let connected = self.link.is_some() && self.amp.alive();
        let Some(t) = self.amp.telemetry.clone().filter(|_| self.amp.alive()) else {
            self.graph_menu = None;
            {
                let full = ui.max_rect();
                let top = ui.cursor().top();
                let row = egui::Rect::from_min_max(egui::pos2(full.left() + 10.0, top), egui::pos2(full.right() - 10.0, top + 22.0));
                self.update_banner(ui, row, full.center().x);
            }
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                match &self.status {
                    LinkStatus::Failed(why) => ui.label(RichText::new(format!("⚠ {why}")).color(RED).size(12.5)),
                    _ => ui.label(RichText::new("Waiting for amplifier…").color(MUTED)),
                };
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, "⚙", "Settings").clicked() {
                    self.show_settings = true;
                }
            });
            return;
        };
        let (fwd_max, fwd_red) = self.amp.model().forward_meter_w();
        let (refl_max, refl_red) = self.amp.model().reflected_meter_w();
        let band = self.amp.current_band().and_then(band_by_number);
        let pk_on = self.cfg.peak_hold;
        let h = move |secs: f32| pk_on.then(|| std::time::Duration::from_secs_f32(secs));
        let temp_c = t.temp1_c();
        let tx = t.mode == AmpMode::OperateTx;
        let carrier = t.forward_w > 0.0 || t.reflected_w > 0.0 || t.input_w > 0.0;
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);

        let kinds = self.cfg.compact_graph_slots;
        let temp_txt = self.fmt_temp(temp_c);
        let swr_alert = self.cfg.swr_alert;
        let mut open_menu = None;
        let mut slot_meter = |ui: &mut egui::Ui, slot: usize| {
            let m = match kinds[slot] {
                CompactGraph::Forward => Meter { label: "FORWARD POWER", value: t.forward_w, min: 0.0, max: fwd_max, text: widgets::rf_text(tx, carrier, format!("{:.0} W", t.forward_w)),
                    zones: Zones::green_red(fwd_red), peak: h(2.5), height: 34.0 },
                CompactGraph::Reflected => Meter { label: "REFLECTED POWER", value: t.reflected_w, min: 0.0, max: refl_max, text: widgets::rf_text(tx, carrier, format!("{:.0} W", t.reflected_w)),
                    zones: Zones::green_red(refl_red), peak: h(2.5), height: 34.0 },
                CompactGraph::Swr => Meter { label: "SWR", value: t.swr.max(1.0), min: 1.0, max: 4.0, text: widgets::rf_text(tx, carrier, format!("{:.2}", t.swr.max(1.0))),
                    zones: Zones { warn: swr_alert, crit: 3.0 }, peak: h(2.5), height: 34.0 },
                CompactGraph::Temp => Meter { label: "PA TEMPERATURE", value: temp_c, min: 0.0, max: 100.0, text: temp_txt.clone(),
                    zones: Zones::green_red(76.0), peak: h(60.0), height: 34.0 },
                CompactGraph::Drive => Meter { label: "DRIVE POWER", value: t.input_w, min: 0.0, max: 100.0, text: widgets::rf_text(tx, carrier, format!("{:.1} W", t.input_w)),
                    zones: Zones { warn: 70.0, crit: 90.0 }, peak: h(2.5), height: 34.0 },
                CompactGraph::DrainCurrent => Meter { label: "PA DRAIN CURRENT", value: t.id1_a, min: 0.0, max: 60.0, text: format!("{:.1} A", t.id1_a),
                    zones: Zones { warn: 45.0, crit: 55.0 }, peak: h(2.5), height: 34.0 },
                CompactGraph::HvVoltage => Meter { label: "PA VOLTAGE (HV)", value: t.hv1_v, min: 0.0, max: 60.0, text: format!("{:.1} V", t.hv1_v),
                    zones: Zones::NONE, peak: h(2.5), height: 34.0 },
            };
            let resp = m.show(ui).on_hover_text("Right-click to change graph");
            if resp.secondary_clicked() {
                let origin = resp.ctx.input(|i| i.viewport().inner_rect).map(|r| r.min).unwrap_or(egui::Pos2::ZERO);
                open_menu = Some((slot, origin + resp.interact_pointer_pos().unwrap_or(resp.rect.center()).to_vec2()));
            }
        };
        slot_meter(ui, 0);
        ui.columns(2, |cols| {
            slot_meter(&mut cols[0], 1);
            slot_meter(&mut cols[1], 2);
        });
        ui.columns(2, |cols| {
            slot_meter(&mut cols[0], 3);
            slot_meter(&mut cols[1], 4);
        });
        if let Some((slot, pos)) = open_menu {
            self.graph_menu = Some((slot, pos, 0, false));
        }
        if let Some(chosen) = self.graph_menu_window(ui.ctx(), kinds) {
            self.cfg.compact_graph_slots = chosen;
            self.cfg.save();
        }

        // Band range and quick controls.
        let (lo, hi) = match (&self.amp.band, band) {
            (Some(b), _) if b.low_khz > 0 => (b.low_khz, b.high_khz),
            (_, Some(b)) => (b.low_khz, b.high_khz),
            _ => (0, 0),
        };
        {
            let full = ui.max_rect();
            let top = ui.cursor().top();
            let row = egui::Rect::from_min_max(egui::pos2(full.left() + 190.0, top), egui::pos2(full.right() - 90.0, top + 22.0));
            self.update_banner(ui, row, full.center().x);
        }
        ui.add_enabled_ui(connected, |ui| {
            ui.horizontal(|ui| {
                let name = band.map(|b| b.name).unwrap_or("–");
                ui.label(RichText::new(format!("{name} · {:.3}–{:.3} MHz", lo as f32 / 1000.0, hi as f32 / 1000.0)).strong().size(11.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let tip = if t.mode.is_operate() { "Switch to standby" } else { "Switch to operate" };
                    let (fill, txt) = if t.mode.is_operate() {
                        (GREEN, "OPERATE")
                    } else {
                        (Color32::from_rgb(255, 140, 0), "STANDBY")
                    };
                    let btn = egui::Button::new(RichText::new(txt).strong().color(Color32::BLACK)).fill(fill);
                    if ui.add(btn).on_hover_text(tip).clicked() {
                        self.send(commands::set_mode(if t.mode.is_operate() { ModeRequest::Standby } else { ModeRequest::OperateRx }));
                    }
                });
            });
        });

        // Status strip: mode, amplifier state, settings.
        let alarm = self.amp.active_codes.first().copied().or((t.alarm_code != 0).then_some(t.alarm_code));
        // Deeper green in light mode so it reads against the strip background.
        let strip_green = if ui.visuals().dark_mode { GREEN } else { Color32::from_rgb(0, 105, 40) };
        let (mode_txt, mode_col) = match t.mode {
            AmpMode::OperateTx => ("TX", RED),
            AmpMode::OperateRx => ("RX", strip_green),
            AmpMode::Standby => ("STANDBY", Color32::from_rgb(255, 140, 0)),
            m => (m.label(), AMBER),
        };
        // Distinct background so the strip reads as a status bar in both themes.
        let strip_fill = if ui.visuals().dark_mode { Color32::from_rgb(33, 40, 54) } else { Color32::from_rgb(201, 210, 224) };
        egui::Frame::new().fill(strip_fill).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if t.mode == AmpMode::OperateTx {
                    widgets::tx_chip(ui, mode_txt);
                } else {
                    widgets::chip(ui, mode_txt, mode_col);
                }
                match alarm.and_then(|c| errors::text(c).map(|x| (c, x))) {
                    Some((c, text)) => ui.label(RichText::new(format!("⚠ {text}")).color(sev_color(errors::severity(c))).size(12.5)),
                    None => ui.label(RichText::new("AMPLIFIER NORMAL").color(strip_green).strong().size(12.5)),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, "⚙", "Settings").clicked() {
                        self.show_settings = true;
                    }
                });
            });
        });
    }

    /// Popup for choosing the graph in a compact slot. It is a borderless window of its own so it
    /// can extend beyond the (small) main window. Returns the new slot assignment when one is picked.
    fn graph_menu_window(&mut self, ctx: &egui::Context, kinds: [CompactGraph; 5]) -> Option<[CompactGraph; 5]> {
        let (slot, pos, frames, seen_focus) = self.graph_menu?;
        let size = egui::vec2(190.0, CompactGraph::ALL.len() as f32 * 26.0 + 14.0);
        let start = clamp_to_monitor(pos, size, ctx.pixels_per_point());
        let builder = egui::ViewportBuilder::default()
            .with_title("Choose graph")
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_inner_size(size)
            .with_position(start);
        let mut chosen = None;
        let mut close = false;
        let mut focused_now = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("graph_menu_viewport"), builder, |ui, _class| {
            focused_now = ui.ctx().input(|i| i.focused);
            if ui.ctx().input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                for k in CompactGraph::ALL {
                    let btn = egui::Button::new(k.label()).selected(kinds[slot] == k).min_size(egui::vec2(ui.available_width(), 22.0));
                    if ui.add(btn).clicked() {
                        let mut n = kinds;
                        // Picking a graph already shown elsewhere swaps the two slots.
                        if let Some(j) = kinds.iter().position(|&x| x == k) {
                            n[j] = kinds[slot];
                        }
                        n[slot] = k;
                        chosen = Some(n);
                        close = true;
                    }
                }
            });
        });
        let seen = seen_focus || focused_now;
        // Dismiss on focus loss (or if it never got focus at all).
        if !focused_now && (seen || frames > 30) {
            close = true;
        }
        self.graph_menu = if close { None } else { Some((slot, pos, frames + 1, seen)) };
        if !close {
            ctx.request_repaint();
        }
        chosen
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            self.settings_start = None;
            if self.settings_dirty {
                self.settings_dirty = false;
                self.cfg.save();
            }
            return;
        }
        let mut open = true;
        let mut f = SettingsFlags::default();
        // A real OS window of its own, so it is not limited by the (possibly tiny) main window.
        let builder = egui::ViewportBuilder::default()
            .with_title("ACOM Controller – Settings")
            .with_icon(crate::icon::window_icon())
            .with_inner_size([760.0, 580.0])
            .with_min_inner_size([620.0, 420.0]);
        let start = *self.settings_start.get_or_insert_with(|| match self.cfg.pos_settings {
            Some(p) => clamp_to_monitor(egui::pos2(p[0], p[1]), egui::vec2(760.0, 580.0), ctx.pixels_per_point()),
            None => centered_pos(ctx, [760.0, 580.0]).unwrap_or(egui::pos2(100.0, 100.0)),
        });
        let builder = builder.with_position(start);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("settings_viewport"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                open = false;
            }
            if let Some(r) = ui.ctx().input(|i| i.viewport().outer_rect) {
                let cur = [r.min.x, r.min.y];
                if r.min.x > -10000.0 && self.cfg.pos_settings != Some(cur) {
                    self.cfg.pos_settings = Some(cur);
                    f.moved = true; // persisted when the window closes
                }
            }
            egui::Panel::left("settings_nav").resizable(false).exact_size(160.0).show(ui, |ui| {
                ui.add_space(8.0);
                for tab in SettingsTab::ALL {
                    let sel = self.settings_tab == tab;
                    let btn = egui::Button::new(RichText::new(tab.label()).size(14.0).strong())
                        .selected(sel)
                        .frame(sel)
                        .min_size(egui::vec2(ui.available_width(), 32.0));
                    if ui.add(btn).clicked() {
                        self.settings_tab = tab;
                    }
                }
            });
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.heading(self.settings_tab.label());
                    ui.add_space(6.0);
                    self.settings_page(ui, &mut f);
                });
            });
            // Esc closes the window.
            if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                open = false;
            }
        });
        self.show_settings = open;
        if f.moved {
            self.settings_dirty = true;
        }
        if f.changed {
            self.settings_dirty = true;
            self.sync_tray(ctx);
            crate::theme::apply(ctx, self.cfg.dark_mode, self.cfg.ui_scale);
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.cfg.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
        }
        if !self.show_settings && self.settings_dirty {
            self.settings_dirty = false;
            self.cfg.save();
        }
        if f.relayout {
            self.apply_layout(ctx);
        }
        if f.restart_radio {
            self.restart_radio(ctx);
        }
        if f.reconnect {
            self.cfg.save();
            self.connect(ctx);
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, f: &mut SettingsFlags) {
        match self.settings_tab {
            SettingsTab::Connection => {
                widgets::card(ui, "Amplifier link", |ui| {
                    ui.horizontal(|ui| {
                        for (c, n) in [(Connection::Serial, "Serial port"), (Connection::Remote, "Remote"), (Connection::Demo, "Demo")] {
                            if ui.selectable_value(&mut self.cfg.connection, c, n).changed() {
                                f.changed = true;
                                f.reconnect = true;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    match self.cfg.connection {
                        Connection::Serial => {
                            ui.horizontal(|ui| {
                                ui.label("COM port");
                                egui::ComboBox::from_id_salt("port")
                                    .selected_text(if self.cfg.com_port.is_empty() { "select…" } else { &self.cfg.com_port })
                                    .show_ui(ui, |ui| {
                                        for p in self.ports.clone() {
                                            if ui.selectable_value(&mut self.cfg.com_port, p.clone(), p).changed() {
                                                f.changed = true;
                                                f.reconnect = true;
                                            }
                                        }
                                    });
                                if ui.button("↻").on_hover_text("Rescan ports").clicked() {
                                    self.refresh_ports();
                                }
                            });
                            ui.label(RichText::new("9600 baud, 8N1 (fixed by the amplifier)").size(12.0).color(MUTED));
                        }
                        Connection::Remote => {
                            ui.horizontal(|ui| {
                                ui.label("Host");
                                f.changed |= ui.text_edit_singleline(&mut self.cfg.remote_host).changed();
                                ui.label("Port");
                                f.changed |= ui.add(egui::DragValue::new(&mut self.cfg.remote_port).range(1..=65535)).changed();
                            });
                            ui.label(RichText::new("Connect to another copy of this program that is sharing its amplifier.").size(12.0).color(MUTED));
                        }
                        Connection::Demo => {
                            ui.label(RichText::new("Built-in simulated amplifier for trying the app without hardware.").color(MUTED));
                        }
                    }
                    ui.add_space(4.0);
                    if ui.button("Apply connection").clicked() {
                        f.reconnect = true;
                    }
                });
                ui.add_space(8.0);
                widgets::card(ui, "Startup and recovery", |ui| {
                    f.changed |= ui.checkbox(&mut self.cfg.auto_connect, "Connect on startup").changed();
                    f.changed |= ui.checkbox(&mut self.cfg.auto_reconnect, "Reconnect automatically after a lost link").changed();
                });
            }
            SettingsTab::PowerSharing => {
                let serial = self.cfg.connection == Connection::Serial;
                if !serial {
                    ui.label(RichText::new("These options apply to a direct serial-port connection.").color(MUTED));
                    ui.add_space(6.0);
                }
                ui.add_enabled_ui(serial, |ui| {
                    widgets::card(ui, "Network sharing", |ui| {
                        f.changed |= ui.checkbox(&mut self.cfg.share_enabled, "Share this amplifier over the network").changed();
                        if self.cfg.share_enabled {
                            ui.horizontal(|ui| {
                                ui.label("TCP port");
                                f.changed |= ui.add(egui::DragValue::new(&mut self.cfg.share_port).range(1024..=65535)).changed();
                            });
                            ui.label(RichText::new("Applies on next connect. No authentication – use only on a trusted network.").size(12.0).color(AMBER));
                        }
                    });
                    ui.add_space(8.0);
                    widgets::card(ui, "Remote power-on", |ui| {
                        f.changed |= ui.checkbox(&mut self.cfg.power_on_enabled, "Enable remote power-on button").changed();
                        if self.cfg.power_on_enabled {
                            ui.horizontal(|ui| {
                                ui.label("Pulse line");
                                for (v, n) in [(PowerPulse::Dtr, "DTR"), (PowerPulse::Rts, "RTS"), (PowerPulse::Both, "DTR+RTS")] {
                                    f.changed |= ui.selectable_value(&mut self.cfg.power_pulse, v, n).changed();
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Pulse length");
                                f.changed |= ui.add(egui::DragValue::new(&mut self.cfg.power_pulse_ms).range(100..=5000).suffix(" ms")).changed();
                                f.changed |= ui.checkbox(&mut self.cfg.power_hold, "Leave asserted").changed();
                            });
                            ui.label(RichText::new("Not described in the protocol document; verify with your amplifier.").size(12.0).color(MUTED));
                        }
                    });
                });
            }
            SettingsTab::Radio => {
                widgets::card(ui, "Frequency source", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (s, n) in [
                            (RadioSource::Off, "Off"),
                            (RadioSource::N1mmUdp, "N1MM+ UDP"),
                            (RadioSource::Rigctld, "rigctld"),
                            (RadioSource::SharedMemory, "Shared memory"),
                        ]
                        .into_iter()
                        // The shared-memory source is Windows-only.
                        .filter(|(s, _)| cfg!(windows) || *s != RadioSource::SharedMemory)
                        {
                            if ui.selectable_value(&mut self.cfg.radio_source, s, n).changed() {
                                f.changed = true;
                                f.restart_radio = true;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    match self.cfg.radio_source {
                        RadioSource::N1mmUdp => {
                            ui.horizontal(|ui| {
                                ui.label("UDP port");
                                f.restart_radio |= ui.add(egui::DragValue::new(&mut self.cfg.n1mm_port).range(1..=65535)).changed();
                            });
                        }
                        RadioSource::Rigctld => {
                            ui.horizontal(|ui| {
                                ui.label("Host");
                                f.restart_radio |= ui.text_edit_singleline(&mut self.cfg.rigctld_host).changed();
                                ui.label("Port");
                                f.restart_radio |= ui.add(egui::DragValue::new(&mut self.cfg.rigctld_port).range(1..=65535)).changed();
                            });
                        }
                        _ => {}
                    }
                    if matches!(self.cfg.radio_source, RadioSource::N1mmUdp | RadioSource::SharedMemory) {
                        ui.horizontal(|ui| {
                            ui.label("Radio");
                            for n in 1..=2u8 {
                                if ui.selectable_value(&mut self.cfg.radio_number, n, format!("Radio {n}")).changed() {
                                    f.restart_radio = true;
                                }
                            }
                        });
                    }
                    f.changed |= f.restart_radio;
                });
                ui.add_space(8.0);
                widgets::card(ui, "Band following", |ui| {
                    f.changed |= ui.checkbox(&mut self.cfg.follow_radio, "Switch amplifier band to follow the radio").changed();
                });
            }
            SettingsTab::Display => {
                widgets::card(ui, "Layout", |ui| {
                    if ui.checkbox(&mut self.cfg.compact, "Compact layout (slim window)").changed() {
                        f.changed = true;
                        f.relayout = true;
                    }
                    ui.horizontal(|ui| {
                        ui.label("UI scale");
                        f.changed |= ui.add(egui::Slider::new(&mut self.cfg.ui_scale, 0.8..=1.6)).changed();
                    });
                    f.changed |= ui.checkbox(&mut self.cfg.dark_mode, "Dark theme").changed();
                });
                ui.add_space(8.0);
                widgets::card(ui, "Readouts", |ui| {
                    f.changed |= ui.checkbox(&mut self.cfg.fahrenheit, "Temperatures in °F").changed();
                    f.changed |= ui.checkbox(&mut self.cfg.peak_hold, "Peak hold on graphs").changed();
                });
            }
            SettingsTab::Alerts => {
                widgets::card(ui, "Warnings", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("SWR alert threshold");
                        f.changed |= ui.add(egui::Slider::new(&mut self.cfg.swr_alert, 1.5..=3.5)).changed();
                    });
                    f.changed |= ui.checkbox(&mut self.cfg.confirm_dangerous, "Ask for confirmation before powering off").changed();
                });
                ui.add_space(8.0);
                widgets::card(ui, "Sound", |ui| {
                    f.changed |= ui.checkbox(&mut self.cfg.sound_enabled, "Play sound alerts").changed();
                    if self.cfg.sound_enabled {
                        ui.horizontal(|ui| {
                            ui.label("Volume");
                            f.changed |= ui.add(egui::Slider::new(&mut self.cfg.sound_volume, 0.05..=1.0)).changed();
                            if ui.button("Test").clicked() {
                                self.sound.play(crate::sound::Alert::Test, self.cfg.sound_volume);
                            }
                        });
                        f.changed |= ui.checkbox(&mut self.cfg.sound_faults, "Warnings and faults").changed();
                        f.changed |= ui.checkbox(&mut self.cfg.sound_swr, "High SWR while transmitting").changed();
                        f.changed |= ui.checkbox(&mut self.cfg.sound_link, "Amplifier link lost / restored").changed();
                    }
                });
            }
            SettingsTab::System => {
                widgets::card(ui, "Window", |ui| {
                    f.changed |= ui.checkbox(&mut self.cfg.always_on_top, "Keep window on top of other windows").changed();
                });
                if crate::tray::SUPPORTED {
                    ui.add_space(8.0);
                    widgets::card(ui, "System tray", |ui| {
                        f.changed |= ui.checkbox(&mut self.cfg.tray_enabled, "Show tray icon").changed();
                        if self.cfg.tray_enabled {
                            f.changed |= ui.checkbox(&mut self.cfg.close_to_tray, "Closing the window hides it to the tray").changed();
                        }
                    });
                }
            }
            SettingsTab::About => {
                widgets::card(ui, "About", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(56.0, 56.0)));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("ACOM Controller").size(22.0).strong());
                            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(MUTED));
                            ui.label("Remote controller for ACOM solid-state amplifiers.");
                        });
                    });
                });
                ui.add_space(8.0);
                widgets::card(ui, "Donate", |ui| {
                    ui.label("If you find this program useful, you can support its development:");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let hand = egui::CursorIcon::PointingHand;
                        let r = ui
                            .add(egui::Image::new(&self.bmc_logo).fit_to_exact_size(egui::vec2(145.0, 41.0)).sense(egui::Sense::click()))
                            .on_hover_cursor(hand)
                            .on_hover_text(BMC_URL);
                        if r.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(BMC_URL));
                        }
                        ui.add_space(8.0);
                        // The PayPal mark is dark blue, so it sits on a white badge to stay readable on any theme.
                        let r = egui::Frame::new()
                            .fill(Color32::WHITE)
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(4.0)
                            .show(ui, |ui| {
                                ui.add(egui::Image::new(&self.paypal_logo).fit_to_exact_size(egui::vec2(90.0, 41.0)).sense(egui::Sense::click()))
                            })
                            .inner
                            .on_hover_cursor(hand)
                            .on_hover_text(PAYPAL_URL);
                        if r.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(PAYPAL_URL));
                        }
                    });
                    ui.add_space(4.0);
                    ui.hyperlink_to("Buy me a coffee", BMC_URL);
                    ui.hyperlink_to("Donate with PayPal", PAYPAL_URL);
                });
                ui.add_space(8.0);
                widgets::card(ui, "Updates", |ui| {
                    ui.horizontal(|ui| {
                        let checking = matches!(self.update, UpdateStatus::Checking);
                        if ui.add_enabled(!checking, egui::Button::new("Check for new version")).clicked() {
                            self.start_update_check(ui.ctx());
                        }
                        match &self.update {
                            UpdateStatus::Idle => {}
                            UpdateStatus::Checking => {
                                ui.add(egui::Spinner::new());
                                ui.label(RichText::new("Checking…").color(MUTED));
                            }
                            UpdateStatus::UpToDate => {
                                ui.label(RichText::new("You are running the latest version.").color(GREEN));
                            }
                            UpdateStatus::Available { version, .. } => {
                                ui.label(RichText::new(format!("Version {version} is available.")).color(AMBER).strong());
                            }
                            UpdateStatus::Failed(why) => {
                                ui.label(RichText::new(format!("Check failed: {why}")).color(RED));
                            }
                        }
                    });
                    if let UpdateStatus::Available { url, .. } = &self.update {
                        ui.hyperlink_to("Open the release page", url.as_str());
                    }
                    let can_upgrade = cfg!(windows)
                        && self.install_rx.is_none()
                        && matches!(&self.update, UpdateStatus::Available { asset: Some(_), .. });
                    if can_upgrade && ui.button("Upgrade now").on_hover_text("Download the new version and restart").clicked() {
                        self.start_install(ui.ctx());
                    }
                    f.changed |= ui.checkbox(&mut self.cfg.check_updates, "Check for a new version when the program starts").changed();
                    ui.label(
                        RichText::new("Contacts GitHub (api.github.com) only; no personal information is sent.").size(12.0).color(MUTED),
                    );
                });
                ui.add_space(8.0);
                widgets::card(ui, "Copyright and license", |ui| {
                    ui.label(RichText::new("Copyright © 2026 Dan Muntz (WD4DAN)").strong());
                    ui.add_space(4.0);
                    ui.label(
                        "This program is free software: you can redistribute it and/or modify it under the terms of the \
                         GNU General Public License as published by the Free Software Foundation, either version 3 of the \
                         License, or (at your option) any later version.",
                    );
                    ui.add_space(4.0);
                    ui.label(
                        "This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; \
                         without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. \
                         See the GNU General Public License for more details.",
                    );
                    ui.add_space(4.0);
                    ui.hyperlink_to("GNU General Public License, version 3", "https://www.gnu.org/licenses/gpl-3.0.html");
                    ui.hyperlink_to("Source code", "https://github.com/danohman/acom-controller");
                });
                ui.add_space(8.0);
                ui.label(
                    RichText::new("ACOM is a trademark of its owner. This is an independent, unofficial program.")
                        .size(12.0)
                        .color(MUTED),
                );
            }
        }
    }

    fn confirmations(&mut self, ctx: &egui::Context) {
        if !self.confirm_power_off {
            return;
        }
        let mut result = None;
        egui::Window::new("Power off amplifier?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("The amplifier will shut down. Remote power-on may not be available.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Confirm").clicked() {
                    result = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
            });
        });
        if let Some(ok) = result {
            self.confirm_power_off = false;
            if ok {
                self.send(commands::set_mode(ModeRequest::TurnOff));
            }
        }
    }
}

impl App {
    /// Tray tooltip and close-to-tray handling.
    fn tray_housekeeping(&mut self, ctx: &egui::Context) {
        let tip = match (&self.amp.telemetry, self.amp.alive()) {
            (Some(t), true) => {
                let band = self.amp.current_band().and_then(band_by_number).map(|b| b.name).unwrap_or("–");
                format!("ACOM · {band} · {:.0} W · SWR {:.2}", t.forward_w, t.swr.max(1.0))
            }
            _ => "ACOM · waiting for amplifier".to_string(),
        };
        let close = ctx.input(|i| i.viewport().close_requested());
        if let Some(t) = self.tray.as_mut() {
            t.set_tooltip(&tip);
            if close && self.cfg.close_to_tray && !t.shared.quit.load(Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                crate::tray::set_visible(ctx, &t.shared, false);
            }
        }
    }
}

/// Clamp a window of `size` points at `pos` (points) into the work area of the monitor holding its centre.
/// Windows only; elsewhere the position is returned unchanged.
fn clamp_to_monitor(pos: egui::Pos2, size: egui::Vec2, ppp: f32) -> egui::Pos2 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        let (w, h) = (size.x * ppp, size.y * ppp);
        let (x, y) = (pos.x * ppp, pos.y * ppp);
        // SAFETY: plain Win32 calls with a zero-initialised, correctly sized MONITORINFO.
        let work = unsafe {
            // Pick the monitor by the window's centre: the outer top-left corner includes the invisible
            // resize border and can lie a few pixels inside the neighbouring monitor.
            let mon = MonitorFromPoint(
                windows_sys::Win32::Foundation::POINT { x: (x + w / 2.0) as i32, y: (y + h / 2.0) as i32 },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut mi) == 0 {
                return pos;
            }
            mi.rcWork
        };
        let nx = x.min(work.right as f32 - w).max(work.left as f32);
        let ny = y.min(work.bottom as f32 - h).max(work.top as f32);
        egui::pos2(nx / ppp, ny / ppp)
    }
    #[cfg(not(windows))]
    {
        let _ = (size, ppp);
        pos
    }
}

/// After a resize to `inner` points, move the window to `target` (or keep it where it is) and make
/// sure it lies inside the monitor's work area.
fn keep_on_screen(ctx: &egui::Context, inner: egui::Vec2, target: Option<egui::Pos2>) {
    let Some((outer, inner_now)) = ctx.input(|i| Some((i.viewport().outer_rect?, i.viewport().inner_rect?))) else { return };
    let size = inner + (outer.size() - inner_now.size());
    let start = target.unwrap_or(outer.min);
    let pos = clamp_to_monitor(start, size, ctx.pixels_per_point());
    if target.is_some() || (pos - outer.min).length() >= 1.0 {
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
    }
}

/// Top-left that centres a window of `size` over the main window.
fn centered_pos(ctx: &egui::Context, size: [f32; 2]) -> Option<egui::Pos2> {
    let c = ctx.input(|i| i.viewport().outer_rect)?.center();
    Some(egui::pos2(c.x - size[0] / 2.0, c.y - size[1] / 2.0))
}

/// Place a child window centred over the main window so it opens on the same monitor.
fn centered_on_main(ctx: &egui::Context, builder: egui::ViewportBuilder, size: [f32; 2]) -> egui::ViewportBuilder {
    match centered_pos(ctx, size) {
        Some(p) => builder.with_position(p),
        None => builder,
    }
}

/// Elapsed seconds as years, months, days, hours and minutes (365-day years, 30-day months),
/// leaving out leading zero units, e.g. "11 months 20 days 4 hours 14 mins".
fn fmt_online(secs: u64) -> String {
    let mins = secs / 60;
    let (years, rest) = (mins / (365 * 1440), mins % (365 * 1440));
    let (months, rest) = (rest / (30 * 1440), rest % (30 * 1440));
    let (days, rest) = (rest / 1440, rest % 1440);
    let (hours, mins) = (rest / 60, rest % 60);
    let units = [(years, "year"), (months, "month"), (days, "day"), (hours, "hour"), (mins, "min")];
    let mut parts: Vec<String> = Vec::new();
    for (n, name) in units {
        if n > 0 || (!parts.is_empty() && name == "min") {
            parts.push(format!("{n} {name}{}", if n == 1 { "" } else { "s" }));
        }
    }
    if parts.is_empty() { "0 mins".to_string() } else { parts.join(" ") }
}

fn sev_color(s: Severity) -> Color32 {
    match s {
        Severity::Info => BLUE,
        Severity::Warning => AMBER,
        Severity::SoftFault | Severity::HardFault => RED,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.track_main_pos(&ctx);
        self.pump(&ctx);
        self.poll_update();
        if self.ports_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) && self.show_settings {
            self.refresh_ports();
        }
        // Keep the UI ticking for staleness detection and reconnect timers.
        ctx.request_repaint_after(Duration::from_millis(250));

        // Compact mode has no top bar; its settings button lives in the status strip.
        if !self.cfg.compact {
            egui::Panel::top("top").show(ui, |ui| {
                ui.add_space(6.0);
                self.top_bar(ui, &ctx);
                ui.add_space(6.0);
            });
        }
        if self.show_log {
            egui::Panel::bottom("log").resizable(true).default_size(120.0).show(ui, |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.comm_log {
                        ui.label(RichText::new(l).monospace().size(11.5).color(MUTED));
                    }
                });
            });
        }
        let mut fit_height = None;
        egui::CentralPanel::default().show(ui, |ui| {
            if self.cfg.compact {
                self.compact_view(ui);
                // Content starts at the top margin; shrink the window to match, no dead space below.
                let r = ui.min_rect();
                fit_height = Some(r.bottom() + 4.0);
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.columns(2, |cols| {
                    self.meters(&mut cols[0]);
                    cols[0].add_space(8.0);
                    self.history(&mut cols[0]);
                    self.controls(&mut cols[1]);
                    cols[1].add_space(8.0);
                    self.alarms(&mut cols[1]);
                    cols[1].add_space(8.0);
                    self.info_card(&mut cols[1]);
                });
            });
        });
        if let Some(h) = fit_height.filter(|_| !self.show_log)
            && let Some(cur) = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()))
            && (cur.y - h).abs() > 1.5
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(420.0, h)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(cur.x, h)));
            keep_on_screen(&ctx, egui::vec2(cur.x, h), None);
        }
        self.settings_window(&ctx);
        self.fault_window(&ctx);
        self.amp_settings_window(&ctx);
        self.confirmations(&ctx);
        self.tray_housekeeping(&ctx);
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(3) {
                egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0]).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(msg));
                });
            } else {
                self.toast = None;
            }
        }
    }

    fn on_exit(&mut self) {
        self.cfg.save();
    }
}

/// Default (inner size, minimum inner size) for the full or compact layout.
pub fn layout_sizes(compact: bool) -> ([f32; 2], [f32; 2]) {
    if compact { ([480.0, 254.0], [420.0, 246.0]) } else { ([1330.0, 815.0], [820.0, 560.0]) }
}
