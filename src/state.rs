//! Aggregated amplifier state, built up from decoded messages.

use crate::protocol::errors::{self, Severity};
use crate::protocol::messages::*;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t: Instant,
    pub forward_w: f32,
    pub reflected_w: f32,
    pub drive_w: f32,
    pub swr: f32,
    pub temp_c: f32,
}

#[derive(Debug, Clone)]
pub struct AlarmEntry {
    pub when: SystemTime,
    pub code: u8,
    pub text: String,
    pub severity: Severity,
    pub active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HardFaultEntry {
    pub record: HardFaultRecord,
    pub snapshot: Option<HardFaultSnapshot>,
}

#[derive(Default)]
pub struct AmpState {
    pub telemetry: Option<Telemetry>,
    pub info: Option<SystemInfo>,
    pub settings: Option<Settings>,
    pub hello: [String; 2],
    pub antenna_names: [String; 4],
    pub band: Option<BandInfo>,
    pub active_codes: Vec<u8>,
    pub atu_serial: Option<String>,
    pub atu_measure: Option<AtuMeasure>,
    pub alarm_log: Vec<AlarmEntry>,
    /// Hard fault records read back from the amplifier, keyed by EEPROM index.
    pub hard_faults: BTreeMap<u8, HardFaultEntry>,
    /// Counters of hard-fault replies received (record, snapshot), used to pace requests.
    pub hf_replies: (u32, u32),
    pub history: VecDeque<Sample>,
    pub last_rx: Option<Instant>,
    pub frames_ok: u64,
    pub peak_forward_w: f32,
    pub peak_at: Option<Instant>,
    pub max_forward_w: f32,
    pub max_swr: f32,
    last_sample: Option<Instant>,
    last_alarm_code: u8,
}

const HISTORY_SPAN: Duration = Duration::from_secs(300);
const PEAK_HOLD: Duration = Duration::from_millis(1500);

impl AmpState {
    pub fn model(&self) -> AmpModel {
        self.info.as_ref().map(|i| i.model).unwrap_or_default()
    }

    /// Current band number: reported band, else derived from the reported band
    /// edges, else from the sensed RF frequency.
    pub fn current_band(&self) -> Option<u8> {
        let max = self.model().max_band();
        if let Some(b) = &self.band {
            if b.band != 0 {
                return Some(b.band);
            }
            let mid = (b.low_khz + b.high_khz) / 2;
            if let Some(x) = crate::protocol::bands::band_for_frequency(mid, max) {
                return Some(x.number);
            }
        }
        let f = self.telemetry.as_ref()?.frequency_khz;
        crate::protocol::bands::band_for_frequency(f, max).map(|b| b.number)
    }

    /// True if telemetry arrived recently.
    pub fn alive(&self) -> bool {
        self.last_rx.is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }

    pub fn reset_link(&mut self) {
        *self = Self { alarm_log: std::mem::take(&mut self.alarm_log), ..Default::default() };
    }

    pub fn reset_session_stats(&mut self) {
        self.max_forward_w = 0.0;
        self.max_swr = 0.0;
        self.history.clear();
    }

    pub fn apply(&mut self, msg: Message) {
        self.last_rx = Some(Instant::now());
        self.frames_ok += 1;
        match msg {
            Message::Telemetry(mut t) => {
                let scale = self.model().current_scale();
                t.id1_a *= scale;
                t.id2_a *= scale;
                self.note_alarm(&t);
                self.record(&t);
                self.telemetry = Some(t);
            }
            Message::SystemInfo(i) => self.info = Some(i),
            Message::Settings(s) => self.settings = Some(s),
            Message::Hello { part, text } => self.hello[part as usize & 1] = text,
            Message::AntennaNames(n) => self.antenna_names = n,
            Message::BandInfo(b) => self.band = Some(b),
            Message::ErrorCodes(c) => {
                for &code in &c {
                    if !self.active_codes.contains(&code) {
                        self.push_alarm(code, true);
                    }
                }
                self.active_codes = c;
            }
            Message::HardFault(r) => {
                self.hf_replies.0 += 1;
                let i = r.index;
                self.hard_faults.entry(i).or_default().record = r;
            }
            Message::HardFaultSnapshot(x) => {
                self.hf_replies.1 += 1;
                let i = x.index;
                self.hard_faults.entry(i).or_default().snapshot = Some(x);
            }
            Message::AtuInfo { serial, .. } => self.atu_serial = Some(serial),
            Message::AtuMeasure(m) => self.atu_measure = Some(m),
            _ => {}
        }
    }

    fn push_alarm(&mut self, code: u8, active: bool) {
        let Some(text) = errors::text(code) else { return };
        self.alarm_log.push(AlarmEntry {
            when: SystemTime::now(),
            code,
            text,
            severity: errors::severity(code),
            active,
        });
        if self.alarm_log.len() > 500 {
            self.alarm_log.remove(0);
        }
    }

    fn note_alarm(&mut self, t: &Telemetry) {
        if t.alarm_code != self.last_alarm_code {
            if t.alarm_code != 0 {
                self.push_alarm(t.alarm_code, true);
            }
            self.last_alarm_code = t.alarm_code;
        }
    }

    fn record(&mut self, t: &Telemetry) {
        let now = Instant::now();
        if t.forward_w >= self.peak_forward_w || self.peak_at.is_none_or(|p| p.elapsed() > PEAK_HOLD) {
            self.peak_forward_w = t.forward_w;
            self.peak_at = Some(now);
        }
        self.max_forward_w = self.max_forward_w.max(t.forward_w);
        if t.forward_w > 5.0 {
            self.max_swr = self.max_swr.max(t.swr);
        }
        if self.last_sample.is_none_or(|l| l.elapsed() >= Duration::from_millis(100)) {
            self.last_sample = Some(now);
            self.history.push_back(Sample {
                t: now,
                forward_w: t.forward_w,
                reflected_w: t.reflected_w,
                drive_w: t.input_w,
                swr: if t.forward_w > 5.0 { t.swr } else { 1.0 },
                temp_c: t.temp1_c(),
            });
            while self.history.front().is_some_and(|s| now.duration_since(s.t) > HISTORY_SPAN) {
                self.history.pop_front();
            }
        }
    }
}
