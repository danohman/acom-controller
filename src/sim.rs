//! Built-in amplifier simulator. Speaks the real wire protocol through the same
//! `Port` interface as a serial device, so the whole stack is exercised in demo mode.

use crate::link::Port;
use crate::protocol::bands::{BANDS, band_by_number};
use crate::protocol::frame::{Decoder, encode};
use crate::protocol::messages::{Telemetry, addr, encode_telemetry};
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Knobs the UI can turn while in demo mode.
#[derive(Default)]
pub struct SimControl {
    pub transmitting: AtomicBool,
    /// Drive power in tenths of a watt.
    pub drive_dw: AtomicU32,
    pub fault: AtomicBool,
}

pub struct Simulator {
    ctl: Arc<SimControl>,
    dec: Decoder,
    out: VecDeque<u8>,
    started: Instant,
    last_tick: Instant,
    mode: u8, // mode nibble
    band: u8,
    tx_allowed: bool,
    temp_c: f32,
    swr: f32,
    rng: u32,
}

impl Simulator {
    pub fn new(ctl: Arc<SimControl>) -> Self {
        Self {
            ctl,
            dec: Decoder::new(),
            out: VecDeque::new(),
            started: Instant::now(),
            last_tick: Instant::now(),
            mode: 5,
            band: 6,
            tx_allowed: true,
            temp_c: 28.0,
            swr: 1.2,
            rng: 0x1234_5678,
        }
    }

    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 1000) as f32 / 1000.0 - 0.5
    }

    fn queue(&mut self, address: u8, data: &[u8]) {
        self.out.extend(encode(address, data));
    }

    fn band_freq(&self) -> u32 {
        band_by_number(self.band).map(|b| (b.low_khz + b.high_khz) / 2).unwrap_or(14100)
    }

    fn telemetry(&mut self) -> Telemetry {
        let operate = self.mode == 6 || self.mode == 7;
        let want_tx = self.ctl.transmitting.load(Ordering::Relaxed) && operate && self.tx_allowed;
        let drive = self.ctl.drive_dw.load(Ordering::Relaxed) as f32 / 10.0;
        let fault = self.ctl.fault.load(Ordering::Relaxed);
        if want_tx && self.mode == 6 {
            self.mode = 7;
        } else if !want_tx && self.mode == 7 {
            self.mode = 6;
        }
        let gain = 12.0;
        let fwd = if self.mode == 7 { (drive * gain + self.noise() * 8.0).clamp(0.0, 720.0) } else { 0.0 };
        let target_swr = 1.23 + if fault { 2.5 } else { 0.0 };
        self.swr += (target_swr - self.swr) * 0.2;
        let refl_frac = ((self.swr - 1.0) / (self.swr + 1.0)).powi(2);
        let refl = fwd * refl_frac;
        let dissip = fwd * 0.45;
        let target_t = 28.0 + dissip * 0.07;
        self.temp_c += (target_t - self.temp_c) * 0.03;
        let mut t = Telemetry {
            mode_byte: self.mode << 4,
            flags1: if operate { (1 << 12) | (1 << 9) } else { 0 },
            flags2: (if self.tx_allowed { 1 << 4 } else { 0 }) | (if operate { (1 << 6) | (1 << 3) } else { 0 }) | (1 << 9),
            clock_s: self.started.elapsed().as_secs() as u32,
            temp1_k: (self.temp_c + 273.15) as u16,
            temp2_k: 0,
            input_w: drive,
            forward_w: fwd,
            reflected_w: refl,
            swr: if fwd > 5.0 { self.swr } else { 1.0 },
            pam1_dissipation_w: dissip,
            pam2_dissipation_w: 0.0,
            band_data_mv: 400 + self.band as u16 * 150,
            vcc5_v: 5.01 + self.noise() * 0.02,
            vcc26_v: 26.2 + self.noise() * 0.1,
            hv1_v: if operate { 50.2 - fwd * 0.004 } else { 0.0 },
            hv2_v: 0.0,
            id1_a: if operate { 1200.0 + fwd * 38.0 } else { 0.0 },
            id2_a: 0.0,
            frequency_khz: self.band_freq(),
            bias_mv: [1480, 1495, 0, 0, 0, 0, 0, 0],
            alarm_code: 0,
            alarm_param: 0,
            fan_speed: if self.temp_c > 45.0 { 3 } else if self.temp_c > 35.0 { 2 } else { 1 },
            lpf_channel: self.band.min(11),
            sys_flags: 0x02,
            ..Default::default()
        };
        if fault {
            t.alarm_code = 0x0D; // PA LOAD SWR TOO HIGH
        }
        t
    }

    fn handle(&mut self, address: u8, data: &[u8]) {
        if address != addr::COMMAND || data.len() < 3 {
            return;
        }
        let (cmd, p1, p2) = (data[0], data[1], data[2]);
        match cmd {
            0x01 => self.reply_message(p2),
            0x02 => match p2 {
                0x05 => self.mode = 5,
                0x06 => self.mode = 6,
                0x07 => self.mode = 7,
                0x0A => self.mode = 5,
                0x40 => self.tx_allowed = false,
                0x80 => self.tx_allowed = true,
                _ => {}
            },
            0x09 => {
                if p2 == 0x8F {
                    self.band = p1.clamp(1, BANDS.len() as u8 - 1);
                }
                self.reply_message(addr::BAND_INFO);
            }
            0x08 => self.ctl.fault.store(false, Ordering::Relaxed),
            _ => {}
        }
    }

    fn reply_message(&mut self, a: u8) {
        match a {
            addr::SYSTEM_INFO => {
                let mut d = vec![1, 3, 9, 2, 0, 1, 0, 1, 0, 0, 0, 1, 2];
                d.extend_from_slice(b"SIM-000001  ");
                d.push(0);
                self.queue(addr::SYSTEM_INFO, &d);
            }
            addr::SETTINGS => self.queue(addr::SETTINGS, &[0x30, 0x10, 0, 0, 0, 0, 0x0A, 0x00, 50]),
            addr::HELLO_1 => self.queue(addr::HELLO_1, b"DEMO AMPLIFIER "),
            addr::HELLO_2 => self.queue(addr::HELLO_2, b"               "),
            addr::BAND_INFO => {
                let b = band_by_number(self.band).copied().unwrap_or(BANDS[5]);
                let mut d = vec![0u8; 17];
                d[2] = b.number;
                d[5..7].copy_from_slice(&(b.low_khz as u16).to_le_bytes());
                d[7..9].copy_from_slice(&(b.high_khz as u16).to_le_bytes());
                self.queue(addr::BAND_INFO, &d);
            }
            addr::ERROR_CODES => self.queue(addr::ERROR_CODES, &[0u8; 22]),
            _ => {}
        }
    }
}

impl Port for Simulator {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.last_tick.elapsed() >= Duration::from_millis(100) {
            self.last_tick = Instant::now();
            let t = self.telemetry();
            self.queue(addr::TELEMETRY, &encode_telemetry(&t));
        }
        if self.out.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
            return Ok(0);
        }
        let n = buf.len().min(self.out.len());
        for slot in buf.iter_mut().take(n) {
            *slot = self.out.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        for f in self.dec.push(data) {
            self.handle(f.address, &f.data);
        }
        Ok(())
    }

    fn name(&self) -> String {
        "Demo amplifier".into()
    }
}
