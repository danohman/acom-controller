//! Hard-fault viewer and amplifier settings editor windows.

use super::{App, sev_color};
use crate::protocol::commands::{self, Beep};
use crate::protocol::errors;
use crate::protocol::messages::{BAUD_TABLE, addr};
use crate::widgets::{AMBER, GREEN, MUTED, RED};
use eframe::egui::{self, RichText};
use std::time::{Duration, Instant};

/// Editable copy of the amplifier's stored settings.
pub struct AmpEdit {
    flags: u16,
    baud: u8,
    iface: u8,
    cmd_set: u8,
    spacing: u16,
    polling: u16,
    buzzer: u8,
    names: [String; 4],
    hello: String,
}

const FLAGS: [(u8, &str); 5] = [
    (0, "Extra cooling"),
    (1, "Auto operate"),
    (2, "Amplifier shows temperature in °F"),
    (3, "CPU buzzer"),
    (4, "ATAC procedure automatic"),
];

const IFACES: [&str; 5] = ["None", "RS-232 levels", "TTL levels", "BCD code", "Analog"];
const CAT_SETS: [&str; 6] = [
    "None",
    "ICOM",
    "YAESU FT-450, FTdx101 …",
    "YAESU FT-817, FT-897 …",
    "YAESU FT-1000MP …",
    "ELECRAFT K3, KENWOOD TS-480 …",
];

impl App {
    /// Read hard-fault records one at a time: select record n, wait for its parts, then move on.
    /// The amplifier pauses its cyclic telemetry while it serves a record, so telemetry is
    /// re-enabled after every step and a silent record is skipped after a timeout.
    pub(super) fn drive_fault_requests(&mut self) {
        let Some(n) = self.hf_next else { return };
        if self.link.is_none() {
            self.hf_next = None;
            return;
        }
        let waited = self.hf_last.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(60));
        let have_record = self.amp.hf_replies.0 > self.hf_base.0;
        let have_snapshot = self.amp.hf_replies.1 > self.hf_base.1;
        match self.hf_step {
            0 => {
                self.hf_base = self.amp.hf_replies;
                self.send(commands::request_hard_fault_record(n));
                self.hf_step = 1;
                self.hf_last = Some(Instant::now());
            }
            1 if have_record && waited > Duration::from_millis(150) => {
                self.send(commands::request_message(addr::HARD_FAULT_2));
                self.hf_step = 2;
                self.hf_last = Some(Instant::now());
            }
            2 if have_snapshot && waited > Duration::from_millis(150) => self.finish_fault_step(n),
            1 | 2 if waited > Duration::from_secs(3) => self.finish_fault_step(n),
            _ => {}
        }
    }

    fn finish_fault_step(&mut self, n: u8) {
        self.send(commands::telemetry_enable(true));
        self.hf_step = 0;
        self.hf_last = Some(Instant::now());
        let total = self.amp.info.as_ref().map(|i| i.hard_fault_count).unwrap_or(0);
        self.hf_next = (n < total).then_some(n + 1);
    }

    pub(super) fn fault_window(&mut self, ctx: &egui::Context) {
        if !self.show_faults {
            return;
        }
        let total = self.amp.info.as_ref().map(|i| i.hard_fault_count).unwrap_or(0);
        let mut open = true;
        egui::Window::new("Hard fault records").open(&mut open).default_size([760.0, 420.0]).resizable(true).show(ctx, |ui| {
            let connected = self.link.is_some() && self.amp.alive();
            ui.horizontal(|ui| {
                ui.label(format!("{total} recorded by the amplifier · {} loaded", self.amp.hard_faults.len()));
                ui.add_enabled_ui(connected && total > 0, |ui| {
                    let label = if self.hf_next.is_some() { "Reading…" } else { "Read records" };
                    if ui.add_enabled(self.hf_next.is_none(), egui::Button::new(label)).clicked() {
                        self.amp.hard_faults.clear();
                        self.hf_step = 0;
                        self.hf_next = Some(1);
                    }
                });
                if self.hf_next.is_some() {
                    ui.spinner();
                }
            });
            ui.label(
                RichText::new("Snapshot of the amplifier taken at each hard fault. The time is the amplifier's internal system counter.")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.separator();
            if self.amp.hard_faults.is_empty() {
                ui.label(RichText::new("No records loaded yet.").color(MUTED));
                return;
            }
            let fah = self.cfg.fahrenheit;
            let any_snap = self.amp.hard_faults.values().any(|e| e.snapshot.is_some());
            egui::ScrollArea::both().max_height(380.0).show(ui, |ui| {
                egui::Grid::new("hf_grid").striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                    let heads: &[&str] = if any_snap {
                        &["#", "Counter", "Faults", "Freq", "Fwd / Refl", "Drive", "26 V · HV", "PA temp"]
                    } else {
                        &["#", "Counter", "Faults"]
                    };
                    for h in heads {
                        ui.label(RichText::new(*h).strong().size(12.0).color(MUTED));
                    }
                    ui.end_row();
                    let mut rows: Vec<_> = self.amp.hard_faults.iter().collect();
                    rows.sort_by_key(|(_, e)| std::cmp::Reverse(e.record.clock_s));
                    for (idx, e) in rows {
                        let r = &e.record;
                        ui.label(format!("{idx}"));
                        let s = r.clock_s;
                        ui.label(format!("{}h {:02}m {:02}s", s / 3600, (s / 60) % 60, s % 60));
                        ui.vertical(|ui| {
                            let codes = errors::active_codes(&r.error_words);
                            if codes.is_empty() {
                                ui.label(RichText::new("–").color(MUTED));
                            }
                            for c in codes {
                                let text = errors::text(c).unwrap_or_else(|| format!("code 0x{c:02X}"));
                                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(sev_color(errors::severity(c)))).wrap_mode(egui::TextWrapMode::Extend));
                            }
                        });
                        match &e.snapshot {
                            _ if !any_snap => {}
                            Some(x) => {
                                ui.label(format!("{:.3} MHz", x.frequency_khz as f32 / 1000.0));
                                ui.label(format!("{} / {} W", x.forward_w, x.reflected_w));
                                ui.label(format!("{:.1} W", x.input_w));
                                ui.label(format!("{:.1} · {:.0} V", x.vcc26_v, x.hv1_v));
                                let c = x.temp1_k as f32 - 273.15;
                                ui.label(if fah { format!("{:.0} °F", c * 9.0 / 5.0 + 32.0) } else { format!("{c:.0} °C") });
                            }
                            None => {
                                for _ in 0..5 {
                                    ui.label(RichText::new("…").color(MUTED));
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if !open {
            self.show_faults = false;
            self.hf_next = None;
        }
    }

    fn new_edit(&self) -> Option<AmpEdit> {
        let s = self.amp.settings.as_ref()?;
        Some(AmpEdit {
            flags: s.user_flags,
            baud: s.cat_baud,
            iface: s.cat_interface,
            cmd_set: s.cat_command_set,
            spacing: s.cat_byte_spacing_us,
            polling: s.cat_polling_ms,
            buzzer: s.buzzer_volume,
            names: self.amp.antenna_names.clone(),
            hello: format!("{}{}", self.amp.hello[0], self.amp.hello[1]).trim_end().to_string(),
        })
    }

    pub(super) fn amp_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_amp_settings {
            return;
        }
        if self.edit.is_none() {
            self.edit = self.new_edit();
        }
        let connected = self.link.is_some() && self.amp.alive();
        let multi_ant = self.amp.telemetry.as_ref().is_some_and(|t| t.atu_assigned());
        let mut open = true;
        let mut sends: Vec<Vec<u8>> = Vec::new();
        let mut refresh = false;
        egui::Window::new("Amplifier settings").open(&mut open).default_width(440.0).resizable(true).show(ctx, |ui| {
            let Some(e) = self.edit.as_mut() else {
                ui.label(RichText::new("Waiting for the amplifier's settings…").color(MUTED));
                return;
            };
            if !connected {
                ui.label(RichText::new("Not connected – changes cannot be sent.").color(AMBER));
            }
            ui.add_enabled_ui(connected, |ui| {
                ui.heading("Operation");
                for (bit, label) in FLAGS {
                    let mut on = e.flags & (1 << bit) != 0;
                    if ui.checkbox(&mut on, label).changed() {
                        e.flags = (e.flags & !(1 << bit)) | ((on as u16) << bit);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Buzzer volume");
                    ui.add(egui::Slider::new(&mut e.buzzer, 0..=100).suffix(" %"));
                    if ui.button("Set & test").clicked() {
                        sends.push(commands::buzzer(Beep::Pulse, e.buzzer));
                        refresh = true;
                    }
                });
                if ui.button("Apply operation settings").clicked() {
                    sends.push(commands::set_user_flags(e.flags));
                    refresh = true;
                }
                ui.separator();

                ui.heading("CAT interface");
                ui.horizontal(|ui| {
                    ui.label("Interface");
                    egui::ComboBox::from_id_salt("cat_if")
                        .selected_text(IFACES.get(e.iface as usize).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, n) in IFACES.iter().enumerate() {
                                ui.selectable_value(&mut e.iface, i as u8, *n);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Command set");
                    egui::ComboBox::from_id_salt("cat_set")
                        .selected_text(CAT_SETS.get(e.cmd_set as usize).copied().unwrap_or("Other"))
                        .show_ui(ui, |ui| {
                            for (i, n) in CAT_SETS.iter().enumerate() {
                                ui.selectable_value(&mut e.cmd_set, i as u8, *n);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Baud rate");
                    let cur = BAUD_TABLE.iter().find(|(c, _)| *c == e.baud).map(|x| x.1).unwrap_or("?");
                    egui::ComboBox::from_id_salt("cat_baud").selected_text(cur).show_ui(ui, |ui| {
                        for (c, n) in BAUD_TABLE {
                            ui.selectable_value(&mut e.baud, c, n);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Byte spacing");
                    ui.add(egui::DragValue::new(&mut e.spacing).range(0..=65535).suffix(" µs"));
                    ui.label("Polling");
                    ui.add(egui::DragValue::new(&mut e.polling).range(0..=65535).suffix(" ms"));
                });
                if ui.button("Apply CAT settings").clicked() {
                    let v = ((e.iface as u16 & 0xF) << 12) | ((e.cmd_set as u16 & 0xF) << 8) | ((e.baud as u16 & 0xF) << 4);
                    sends.push(commands::set_cat_settings_1(v));
                    sends.push(commands::set_cat_byte_spacing_us(e.spacing));
                    sends.push(commands::set_cat_polling_ms(e.polling));
                    refresh = true;
                }
                ui.separator();

                ui.heading("User text");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut e.hello).char_limit(30).desired_width(260.0));
                    if ui.button("Apply").clicked() {
                        sends.extend(commands::hello_message(&e.hello));
                        refresh = true;
                    }
                });
                ui.label(RichText::new("Up to 30 characters, shown on the amplifier (for example a callsign).").size(12.0).color(MUTED));

                if multi_ant {
                    ui.separator();
                    ui.heading("Antenna names");
                    for (i, n) in e.names.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Antenna {}", i + 1));
                            ui.add(egui::TextEdit::singleline(n).char_limit(12).desired_width(140.0));
                        });
                    }
                    if ui.button("Apply antenna names").clicked() {
                        sends.push(commands::antenna_names(&e.names));
                        refresh = true;
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Changes are written to the amplifier's memory.").size(12.0).color(if connected { GREEN } else { RED }));
        });
        for w in sends {
            self.send(w);
        }
        if refresh {
            for a in [addr::SETTINGS, addr::HELLO_1, addr::HELLO_2, addr::ANT_NAMES] {
                self.send(commands::request_message(a));
            }
            self.say("Settings sent to the amplifier");
        }
        if !open {
            self.show_amp_settings = false;
            self.edit = None;
        }
    }
}
