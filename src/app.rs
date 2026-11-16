//! Main application window.

use crate::config::{Config, Connection, PowerPulse, RadioSource};
use crate::link::{self, Cmd, Event, LinkHandle};
use crate::protocol::bands::{BANDS, band_by_number, band_for_frequency};
use crate::protocol::commands::{self, AtuCommand, ModeRequest, Select};
use crate::protocol::errors::{self, Severity};
use crate::protocol::messages::{AmpMode, BAUD_TABLE, addr, antenna_type_text, atac_text, atu_label_text};
use crate::radio::{self, RadioEvent, RadioHandle};
use crate::state::AmpState;
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

enum Pending {
    TurnOff,
    Restart,
    TunePrompt,
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
    show_log: bool,
    pending: Option<Pending>,
    comm_log: VecDeque<String>,
    toast: Option<(String, Instant)>,
    plot_metric: usize,
    settings_dirty: bool,
    last_info_req: Option<Instant>,
    logo: egui::TextureHandle,
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
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
            show_log: false,
            pending: None,
            comm_log: VecDeque::new(),
            toast: None,
            plot_metric: 0,
            settings_dirty: false,
            last_info_req: None,
            logo: crate::icon::texture(&cc.egui_ctx),
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
            show_amp_settings: false,
            edit: None,
        };
        app.sync_tray(&cc.egui_ctx);
        app.refresh_ports();
        if app.cfg.auto_connect {
            app.connect(&cc.egui_ctx);
        }
        app.restart_radio(&cc.egui_ctx);
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
        if let Some(t) = self.next_retry {
            if Instant::now() >= t && self.link.is_none() {
                self.next_retry = None;
                self.connect(ctx);
            }
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
        if self.amp.antenna_names.iter().all(|n| n.is_empty()) {
            want.push(addr::ANT_NAMES);
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
            self.send(commands::select(Select::Band(target.number)));
        }
    }

    /// Create or drop the tray icon to match the setting.
    fn sync_tray(&mut self, ctx: &egui::Context) {
        if self.cfg.tray_enabled && self.tray.is_none() {
            self.tray = crate::tray::Tray::new(ctx);
        } else if !self.cfg.tray_enabled {
            if let Some(t) = self.tray.take() {
                crate::tray::set_visible(ctx, &t.shared, true);
            }
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
            if let Some(t) = self.amp.telemetry.as_ref().filter(|_| !self.cfg.compact) {
                if self.amp.alive() {
                    let (txt, col) = match t.mode {
                        AmpMode::OperateTx => (t.mode.label(), RED),
                        AmpMode::OperateRx => (t.mode.label(), GREEN),
                        AmpMode::Standby => (t.mode.label(), BLUE),
                        m => (m.label(), AMBER),
                    };
                    widgets::chip(ui, txt, col);
                }
            }
            if self.clients > 0 {
                widgets::chip(ui, &format!("{} remote client(s)", self.clients), BLUE);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let gear = if self.cfg.compact {
                    widgets::icon_button(ui, "⚙", "Settings")
                } else {
                    ui.button("⚙ Settings")
                };
                if gear.clicked() {
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
                            self.pending = Some(Pending::TurnOff);
                        } else {
                            self.send(commands::set_mode(ModeRequest::TurnOff));
                        }
                    }
                } else if self.cfg.power_on_enabled && self.cfg.connection == Connection::Serial {
                    if ui.button("Power on").on_hover_text("Pulse the RS-232 control line configured in Settings").clicked() {
                        self.send_power_pulse(ctx);
                    }
                }
            });
        });
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
        let fs = model.full_scale_w();
        let peak = self.cfg.peak_hold.then_some(self.amp.peak_forward_w);
        widgets::card(ui, "RF power", |ui| {
            ui.horizontal(|ui| {
                widgets::readout(ui, "FORWARD", &format!("{:.0} W", t.forward_w), Color32::WHITE);
                ui.add_space(18.0);
                widgets::readout(ui, "REFLECTED", &format!("{:.0} W", t.reflected_w), if t.reflected_w > 20.0 { AMBER } else { Color32::WHITE });
                ui.add_space(18.0);
                let swr_col = if t.swr >= 3.0 { RED } else if t.swr >= self.cfg.swr_alert { AMBER } else { GREEN };
                widgets::readout(ui, "SWR", &format!("{:.2}", t.swr.max(1.0)), swr_col);
                ui.add_space(18.0);
                widgets::readout(ui, "DRIVE", &format!("{:.1} W", t.input_w), Color32::WHITE);
            });
            ui.add_space(6.0);
            Meter {
                label: "Forward power",
                value: t.forward_w,
                min: 0.0,
                max: fs,
                text: format!("{:.0} W", t.forward_w),
                zones: Zones { warn: fs * 0.85, crit: fs * 0.97 },
                peak,
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "Reflected power",
                value: t.reflected_w,
                min: 0.0,
                max: fs * 0.2,
                text: format!("{:.0} W", t.reflected_w),
                zones: Zones { warn: fs * 0.03, crit: fs * 0.08 },
                peak: None,
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "SWR",
                value: t.swr.max(1.0),
                min: 1.0,
                max: 4.0,
                text: format!("{:.2}", t.swr.max(1.0)),
                zones: Zones { warn: self.cfg.swr_alert, crit: 3.0 },
                peak: None,
                height: 40.0,
            }
            .show(ui);
            Meter {
                label: "Drive power",
                value: t.input_w,
                min: 0.0,
                max: 100.0,
                text: format!("{:.1} W", t.input_w),
                zones: Zones { warn: 70.0, crit: 90.0 },
                peak: None,
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
                min: 20.0,
                max: 100.0,
                text: self.fmt_temp(temp),
                zones: Zones { warn: 60.0, crit: 80.0 },
                peak: None,
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
                peak: None,
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
                peak: None,
                height: 40.0,
            }
            .show(ui);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                widgets::readout(ui, "26 V RAIL", &format!("{:.1} V", t.vcc26_v), Color32::WHITE);
                ui.add_space(14.0);
                widgets::readout(ui, "5 V RAIL", &format!("{:.2} V", t.vcc5_v), Color32::WHITE);
                ui.add_space(14.0);
                widgets::readout(ui, "DISSIPATION", &format!("{:.0} W", t.dissipation_w()), Color32::WHITE);
                ui.add_space(14.0);
                widgets::readout(ui, "FAN", &format!("{}/4", t.fan_speed), Color32::WHITE);
                ui.add_space(14.0);
                widgets::readout(
                    ui,
                    "LPF",
                    &if t.lpf_channel == 0 { "–".into() } else { t.lpf_channel.to_string() },
                    if t.lpf_tuned() || t.lpf_channel == 0 { Color32::WHITE } else { AMBER },
                );
            });
        });
        let _ = model;
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
                    let label = if tx_allowed { "Inhibit TX" } else { "Allow TX" };
                    if ui.button(label).on_hover_text("Prohibit / allow entering OPERATE/TX").clicked() {
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

        widgets::card(ui, "Band & antenna", |ui| {
            let max = self.amp.model().max_band();
            let cur_band = self.amp.current_band();
            // Antenna/segment selection only exists with an ATU/ASEL; a bare 1200S has one port.
            let multi_ant = self.amp.telemetry.as_ref().is_some_and(|t| t.atu_assigned());
            ui.add_enabled_ui(connected, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for b in BANDS.iter().filter(|b| b.number <= max) {
                        let sel = Some(b.number) == cur_band;
                        let mut txt = RichText::new(b.name.replace(' ', "")).size(13.0);
                        let mut btn = egui::Button::new(egui::RichText::new("")).min_size(egui::vec2(52.0, 28.0));
                        if sel {
                            txt = txt.strong().color(egui::Color32::BLACK);
                            btn = egui::Button::new(txt).min_size(egui::vec2(52.0, 28.0)).fill(GREEN);
                        } else {
                            btn = egui::Button::new(txt).min_size(egui::vec2(52.0, 28.0));
                        }
                        if ui.add(btn).clicked() {
                            self.send(commands::select(Select::Band(b.number)));
                        }
                    }
                });
                if multi_ant {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Antenna");
                        let cur = self.amp.band.as_ref().map(|b| b.antenna).unwrap_or(0);
                        for n in 1..=4u8 {
                            let name = self.amp.antenna_names.get(n as usize - 1).map(|s| s.as_str()).unwrap_or("");
                            let label = if name.is_empty() { format!("A{n}") } else { format!("{n}·{name}") };
                            let btn = egui::Button::new(label).selected(cur == n);
                            if ui.add(btn).clicked() {
                                self.send(commands::select(Select::Antenna(n)));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Segment");
                        if ui.button("◀").clicked() {
                            self.send(commands::select(Select::PrevSegment));
                        }
                        if ui.button("▶").clicked() {
                            self.send(commands::select(Select::NextSegment));
                        }
                    });
                }
            });
            if let Some(b) = &self.amp.band {
                ui.add_space(4.0);
                let name = self.amp.current_band().and_then(band_by_number).map(|x| x.name).unwrap_or("?");
                let mut line = format!("{} · {}–{} kHz", name, b.low_khz, b.high_khz);
                if multi_ant {
                    line += &format!(" · antenna {} ({}) · segment {}/{}", b.antenna, antenna_type_text(b.antenna_type), b.segment, b.segment_count);
                }
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
                    Color32::WHITE,
                );
                if self.cfg.radio_source != RadioSource::Off {
                    ui.add_space(18.0);
                    widgets::readout(
                        ui,
                        "RADIO",
                        &self.radio_khz.map(|k| format!("{:.3} MHz", k as f32 / 1000.0)).unwrap_or("–".into()),
                        Color32::WHITE,
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

        // ATU appears only when one is assigned.
        if self.amp.telemetry.as_ref().is_some_and(|t| t.atu_assigned()) {
            widgets::card(ui, "ATU", |ui| {
                let label = self.amp.band.as_ref().map(|b| atu_label_text(b.atu_label)).unwrap_or("");
                if !label.is_empty() {
                    ui.label(label);
                }
                if let Some(t) = &self.amp.telemetry {
                    if let Some(s) = atac_text(t.mode_byte) {
                        ui.label(RichText::new(s).color(AMBER));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Full tune").clicked() {
                        self.pending = Some(Pending::TunePrompt);
                    }
                    if ui.button("Stop tune").clicked() {
                        self.send(commands::atu(AtuCommand::StopTune));
                    }
                    if ui.button("Bypass").clicked() {
                        let a = self.amp.band.as_ref().map(|b| b.antenna).unwrap_or(1);
                        self.send(commands::atu(AtuCommand::Bypass(a)));
                    }
                });
            });
            ui.add_space(8.0);
        }

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
                if let Some(t) = &self.amp.telemetry {
                    if t.alarm_code != 0 && !v.contains(&t.alarm_code) {
                        v.push(t.alarm_code);
                    }
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
                let h = t.clock_s / 3600;
                ui.label(RichText::new(format!("Uptime {}h {:02}m", h, (t.clock_s / 60) % 60)).size(12.0).color(MUTED));
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
    fn apply_layout(&self, ctx: &egui::Context) {
        let (size, min) = layout_sizes(self.cfg.compact);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min.into()));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
    }

    /// Slim layout: everything an operator glances at, in one narrow column.
    fn compact_view(&mut self, ui: &mut egui::Ui) {
        let connected = self.link.is_some() && self.amp.alive();
        let Some(t) = self.amp.telemetry.clone().filter(|_| self.amp.alive()) else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new("Waiting for amplifier…").color(MUTED));
            });
            return;
        };
        let fs = self.amp.model().full_scale_w();
        let band = self.amp.current_band().and_then(band_by_number);
        // Antenna selection only exists with an ATU/ASEL; a single-port amp has none.
        let multi_ant = t.atu_assigned();
        let peak = self.cfg.peak_hold.then_some(self.amp.peak_forward_w);
        let temp_c = t.temp1_c();
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);

        Meter { label: "FORWARD POWER", value: t.forward_w, min: 0.0, max: fs, text: format!("{:.0} W", t.forward_w),
            zones: Zones { warn: fs * 0.85, crit: fs * 0.97 }, peak, height: 34.0 }.show(ui);
        ui.columns(2, |cols| {
            Meter { label: "PA TEMPERATURE", value: temp_c, min: 20.0, max: 100.0, text: self.fmt_temp(temp_c),
                zones: Zones { warn: 70.0, crit: 85.0 }, peak: None, height: 34.0 }.show(&mut cols[0]);
            Meter { label: "REFLECTED POWER", value: t.reflected_w, min: 0.0, max: fs * 0.2, text: format!("{:.0} W", t.reflected_w),
                zones: Zones { warn: fs * 0.03, crit: fs * 0.08 }, peak: None, height: 34.0 }.show(&mut cols[1]);
        });

        // Band range and quick controls.
        let (lo, hi) = match (&self.amp.band, band) {
            (Some(b), _) if b.low_khz > 0 => (b.low_khz, b.high_khz),
            (_, Some(b)) => (b.low_khz, b.high_khz),
            _ => (0, 0),
        };
        ui.add_enabled_ui(connected, |ui| {
            ui.horizontal(|ui| {
                let name = band.map(|b| b.name).unwrap_or("–");
                ui.label(RichText::new(format!("{name} · {:.3}–{:.3} MHz", lo as f32 / 1000.0, hi as f32 / 1000.0)).strong().size(14.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if multi_ant && ui.button("ANT").on_hover_text("Next antenna").clicked() {
                        self.send(commands::select(Select::NextAntenna));
                    }
                    let tip = if t.mode.is_operate() { "Switch to standby" } else { "Switch to operate" };
                    if ui.button(RichText::new("OPR/STB").strong()).on_hover_text(tip).clicked() {
                        self.send(commands::set_mode(if t.mode.is_operate() { ModeRequest::Standby } else { ModeRequest::OperateRx }));
                    }
                });
            });
        });

        // Status strip: mode, amplifier state, SWR, drive.
        let alarm = self.amp.active_codes.first().copied().or((t.alarm_code != 0).then_some(t.alarm_code));
        let (mode_txt, mode_col) = match t.mode {
            AmpMode::OperateTx => (t.mode.label(), RED),
            AmpMode::OperateRx => (t.mode.label(), GREEN),
            AmpMode::Standby => (t.mode.label(), BLUE),
            m => (m.label(), AMBER),
        };
        let swr = t.swr.max(1.0);
        let swr_col = if swr >= 3.0 { RED } else if swr >= self.cfg.swr_alert { AMBER } else { GREEN };
        ui.horizontal(|ui| {
            widgets::chip(ui, mode_txt, mode_col);
            match alarm.and_then(|c| errors::text(c).map(|x| (c, x))) {
                Some((c, text)) => ui.label(RichText::new(format!("⚠ {text}")).color(sev_color(errors::severity(c))).size(12.5)),
                None => ui.label(RichText::new("AMPLIFIER NORMAL").color(GREEN).strong().size(12.5)),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.1} W", t.input_w)).monospace().strong());
                ui.label(RichText::new("DRIVE").size(11.5).color(MUTED));
                ui.add_space(6.0);
                ui.label(RichText::new(format!("{swr:.2}")).monospace().strong().color(swr_col));
                ui.label(RichText::new("SWR").size(11.5).color(MUTED));
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut changed = false;
        let mut reconnect = false;
        let mut restart_radio = false;
        let mut relayout = false;
        egui::Window::new("Settings").open(&mut open).default_size([440.0, 700.0]).resizable(true).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Connection");
                ui.horizontal(|ui| {
                    for (c, n) in [(Connection::Serial, "Serial port"), (Connection::Remote, "Remote"), (Connection::Demo, "Demo")] {
                        if ui.selectable_value(&mut self.cfg.connection, c, n).changed() {
                            changed = true;
                            reconnect = true;
                        }
                    }
                });
                match self.cfg.connection {
                    Connection::Serial => {
                        ui.horizontal(|ui| {
                            ui.label("COM port");
                            egui::ComboBox::from_id_salt("port")
                                .selected_text(if self.cfg.com_port.is_empty() { "select…" } else { &self.cfg.com_port })
                                .show_ui(ui, |ui| {
                                    for p in self.ports.clone() {
                                        if ui.selectable_value(&mut self.cfg.com_port, p.clone(), p).changed() {
                                            changed = true;
                                            reconnect = true;
                                        }
                                    }
                                });
                            if ui.button("↻").on_hover_text("Rescan ports").clicked() {
                                self.refresh_ports();
                            }
                        });
                        ui.label(RichText::new("9600 baud, 8N1 (fixed by the amplifier)").size(12.0).color(MUTED));
                        changed |= ui.checkbox(&mut self.cfg.share_enabled, "Share this amplifier over the network").changed();
                        if self.cfg.share_enabled {
                            ui.horizontal(|ui| {
                                ui.label("TCP port");
                                changed |= ui.add(egui::DragValue::new(&mut self.cfg.share_port).range(1024..=65535)).changed();
                            });
                            ui.label(RichText::new("Applies on next connect. No authentication – use only on a trusted network.").size(12.0).color(AMBER));
                        }
                        changed |= ui.checkbox(&mut self.cfg.power_on_enabled, "Enable remote power-on button").changed();
                        if self.cfg.power_on_enabled {
                            ui.horizontal(|ui| {
                                ui.label("Pulse line");
                                for (v, n) in [(PowerPulse::Dtr, "DTR"), (PowerPulse::Rts, "RTS"), (PowerPulse::Both, "DTR+RTS")] {
                                    changed |= ui.selectable_value(&mut self.cfg.power_pulse, v, n).changed();
                                }
                                changed |= ui.add(egui::DragValue::new(&mut self.cfg.power_pulse_ms).range(100..=5000).suffix(" ms")).changed();
                                changed |= ui.checkbox(&mut self.cfg.power_hold, "Leave asserted").changed();
                            });
                            ui.label(RichText::new("Not described in the protocol document; verify with your amplifier.").size(12.0).color(MUTED));
                        }
                    }
                    Connection::Remote => {
                        ui.horizontal(|ui| {
                            ui.label("Host");
                            changed |= ui.text_edit_singleline(&mut self.cfg.remote_host).changed();
                            ui.label("Port");
                            changed |= ui.add(egui::DragValue::new(&mut self.cfg.remote_port).range(1..=65535)).changed();
                        });
                    }
                    Connection::Demo => {
                        ui.label(RichText::new("Built-in simulated amplifier for trying the app without hardware.").color(MUTED));
                    }
                }
                changed |= ui.checkbox(&mut self.cfg.auto_connect, "Connect on startup").changed();
                changed |= ui.checkbox(&mut self.cfg.auto_reconnect, "Reconnect automatically").changed();
                if ui.button("Apply connection").clicked() {
                    reconnect = true;
                }
                ui.separator();

                ui.heading("Radio integration");
                ui.horizontal(|ui| {
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
                            changed = true;
                            restart_radio = true;
                        }
                    }
                });
                match self.cfg.radio_source {
                    RadioSource::N1mmUdp => {
                        ui.horizontal(|ui| {
                            ui.label("UDP port");
                            restart_radio |= ui.add(egui::DragValue::new(&mut self.cfg.n1mm_port).range(1..=65535)).changed();
                        });
                    }
                    RadioSource::Rigctld => {
                        ui.horizontal(|ui| {
                            restart_radio |= ui.text_edit_singleline(&mut self.cfg.rigctld_host).changed();
                            restart_radio |= ui.add(egui::DragValue::new(&mut self.cfg.rigctld_port).range(1..=65535)).changed();
                        });
                    }
                    _ => {}
                }
                if matches!(self.cfg.radio_source, RadioSource::N1mmUdp | RadioSource::SharedMemory) {
                    ui.horizontal(|ui| {
                        ui.label("Radio");
                        for n in 1..=2u8 {
                            if ui.selectable_value(&mut self.cfg.radio_number, n, format!("Radio {n}")).changed() {
                                restart_radio = true;
                            }
                        }
                    });
                }
                changed |= restart_radio;
                changed |= ui.checkbox(&mut self.cfg.follow_radio, "Switch amplifier band to follow the radio").changed();
                ui.separator();

                ui.heading("Display");
                if ui.checkbox(&mut self.cfg.compact, "Compact layout (slim window)").changed() {
                    changed = true;
                    relayout = true;
                }
                changed |= ui.checkbox(&mut self.cfg.dark_mode, "Dark theme").changed();
                changed |= ui.checkbox(&mut self.cfg.fahrenheit, "Temperatures in °F").changed();
                changed |= ui.checkbox(&mut self.cfg.peak_hold, "Peak hold on power meter").changed();
                changed |= ui.checkbox(&mut self.cfg.confirm_dangerous, "Confirm power off").changed();
                ui.horizontal(|ui| {
                    ui.label("SWR alert");
                    changed |= ui.add(egui::Slider::new(&mut self.cfg.swr_alert, 1.5..=3.5)).changed();
                });
                ui.horizontal(|ui| {
                    ui.label("UI scale");
                    changed |= ui.add(egui::Slider::new(&mut self.cfg.ui_scale, 0.8..=1.6)).changed();
                });
                ui.separator();

                ui.heading("Sound alerts");
                changed |= ui.checkbox(&mut self.cfg.sound_enabled, "Play sound alerts").changed();
                if self.cfg.sound_enabled {
                    ui.horizontal(|ui| {
                        ui.label("Volume");
                        changed |= ui.add(egui::Slider::new(&mut self.cfg.sound_volume, 0.05..=1.0)).changed();
                        if ui.button("Test").clicked() {
                            self.sound.play(crate::sound::Alert::Test, self.cfg.sound_volume);
                        }
                    });
                    changed |= ui.checkbox(&mut self.cfg.sound_faults, "Warnings and faults").changed();
                    changed |= ui.checkbox(&mut self.cfg.sound_swr, "High SWR while transmitting").changed();
                    changed |= ui.checkbox(&mut self.cfg.sound_link, "Amplifier link lost / restored").changed();
                }
                if crate::tray::SUPPORTED {
                    ui.separator();
                    ui.heading("System tray");
                    changed |= ui.checkbox(&mut self.cfg.tray_enabled, "Show tray icon").changed();
                    if self.cfg.tray_enabled {
                        changed |= ui.checkbox(&mut self.cfg.close_to_tray, "Closing the window hides it to the tray").changed();
                    }
                }
                if changed {
                    self.settings_dirty = true;
                }
            });
        });
        self.show_settings = open;
        if changed {
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
        if relayout {
            self.apply_layout(ctx);
        }
        if restart_radio {
            self.restart_radio(ctx);
        }
        if reconnect {
            self.cfg.save();
            self.connect(ctx);
        }
    }

    fn confirmations(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pending else { return };
        let (title, body) = match p {
            Pending::TurnOff => ("Power off amplifier?", "The amplifier will shut down. Remote power-on may not be available."),
            Pending::Restart => ("Restart amplifier?", "The amplifier will reboot and re-initialise."),
            Pending::TunePrompt => (
                "Start ATU full tune?",
                "The transceiver must supply drive power during tuning, then remove it when instructed. Use low power.",
            ),
        };
        let mut result = None;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(body);
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
            let p = self.pending.take();
            if ok {
                match p {
                    Some(Pending::TurnOff) => self.send(commands::set_mode(ModeRequest::TurnOff)),
                    Some(Pending::Restart) => self.send(commands::set_mode(ModeRequest::Restart)),
                    Some(Pending::TunePrompt) => self.send(commands::atu(AtuCommand::FullTune)),
                    None => {}
                }
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
        self.pump(&ctx);
        if self.ports_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) && self.show_settings {
            self.refresh_ports();
        }
        // Keep the UI ticking for staleness detection and reconnect timers.
        ctx.request_repaint_after(Duration::from_millis(250));

        egui::Panel::top("top").show_inside(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui, &ctx);
            ui.add_space(6.0);
        });
        if self.show_log {
            egui::Panel::bottom("log").resizable(true).default_size(120.0).show_inside(ui, |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.comm_log {
                        ui.label(RichText::new(l).monospace().size(11.5).color(MUTED));
                    }
                });
            });
        }
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.cfg.compact {
                self.compact_view(ui);
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
        self.settings_window(&ctx);
        self.fault_window(&ctx);
        self.amp_settings_window(&ctx);
        self.confirmations(&ctx);
        self.tray_housekeeping(&ctx);
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(3) {
                egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0]).show(&ctx, |ui| {
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
    if compact { ([480.0, 174.0], [420.0, 166.0]) } else { ([1330.0, 815.0], [820.0, 560.0]) }
}
