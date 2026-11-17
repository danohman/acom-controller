//! Transceiver frequency sources. Each source runs on its own thread and
//! reports the radio's frequency in kHz so the amplifier band can follow it.

use crate::config::{Config, RadioSource};
use crossbeam_channel::{Receiver, Sender};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RadioEvent {
    Frequency(u32),
    Status(String),
}

pub struct RadioHandle {
    pub rx: Receiver<RadioEvent>,
    stop: Arc<AtomicBool>,
}

impl Drop for RadioHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(cfg: &Config, ctx: eframe::egui::Context) -> Option<RadioHandle> {
    if cfg.radio_source == RadioSource::Off {
        return None;
    }
    let (tx, rx) = crossbeam_channel::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let (c, s) = (cfg.clone(), stop.clone());
    std::thread::Builder::new()
        .name("radio".into())
        .spawn(move || match c.radio_source {
            RadioSource::N1mmUdp => n1mm(&c, &tx, &s, &ctx),
            RadioSource::Rigctld => rigctld(&c, &tx, &s, &ctx),
            RadioSource::SharedMemory => shared_memory(&c, &tx, &s, &ctx),
            RadioSource::Off => {}
        })
        .ok()?;
    Some(RadioHandle { rx, stop })
}

fn emit(tx: &Sender<RadioEvent>, ctx: &eframe::egui::Context, e: RadioEvent) {
    let _ = tx.send(e);
    ctx.request_repaint();
}

/// Extract the text of `<tag>…</tag>`.
fn xml_field<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find('<')? + start;
    Some(xml[start..end].trim())
}

/// Parse an N1MM+ RadioInfo datagram: returns (radio number, kHz).
pub fn parse_n1mm(xml: &str) -> Option<(u8, u32)> {
    let radio: u8 = xml_field(xml, "RadioNr")?.parse().ok()?;
    // TXFreq is in units of 10 Hz.
    let tx: u64 = xml_field(xml, "TXFreq")?.parse().ok()?;
    Some((radio, (tx / 100) as u32))
}

fn n1mm(cfg: &Config, tx: &Sender<RadioEvent>, stop: &AtomicBool, ctx: &eframe::egui::Context) {
    let sock = match UdpSocket::bind(("0.0.0.0", cfg.n1mm_port)) {
        Ok(s) => s,
        Err(e) => return emit(tx, ctx, RadioEvent::Status(format!("UDP {}: {e}", cfg.n1mm_port))),
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    emit(tx, ctx, RadioEvent::Status(format!("Listening on UDP {}", cfg.n1mm_port)));
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        if let Ok(n) = sock.recv(&mut buf)
            && let Some((radio, khz)) = parse_n1mm(&String::from_utf8_lossy(&buf[..n]))
            && radio == cfg.radio_number
            && khz > 0
        {
            emit(tx, ctx, RadioEvent::Frequency(khz));
        }
    }
}

fn rigctld(cfg: &Config, tx: &Sender<RadioEvent>, stop: &AtomicBool, ctx: &eframe::egui::Context) {
    let addr = format!("{}:{}", cfg.rigctld_host, cfg.rigctld_port);
    while !stop.load(Ordering::Relaxed) {
        match TcpStream::connect(&addr) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                emit(tx, ctx, RadioEvent::Status(format!("Connected to {addr}")));
                let Ok(mut w) = s.try_clone() else {
                    emit(tx, ctx, RadioEvent::Status("rigctld: cannot duplicate socket".into()));
                    pause(stop);
                    continue;
                };
                let mut r = BufReader::new(s);
                let mut line = String::new();
                while !stop.load(Ordering::Relaxed) {
                    if w.write_all(b"f\n").is_err() {
                        break;
                    }
                    line.clear();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if let Ok(hz) = line.trim().parse::<u64>()
                        && hz > 0
                    {
                        emit(tx, ctx, RadioEvent::Frequency((hz / 1000) as u32));
                    }
                    std::thread::sleep(Duration::from_millis(400));
                }
                emit(tx, ctx, RadioEvent::Status("rigctld connection lost".into()));
            }
            Err(e) => emit(tx, ctx, RadioEvent::Status(format!("{addr}: {e}"))),
        }
        pause(stop);
    }
}

/// Wait 5 s before reconnecting, waking early when asked to stop.
fn pause(stop: &AtomicBool) {
    for _ in 0..20 {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn shared_memory(cfg: &Config, tx: &Sender<RadioEvent>, stop: &AtomicBool, ctx: &eframe::egui::Context) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{FILE_MAP_READ, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile};

    let name: Vec<u16> = "DXLog.net.mmf\0".encode_utf16().collect();
    let offset = if cfg.radio_number == 2 { 30 } else { 0 };
    let mut last = 0u32;
    while !stop.load(Ordering::Relaxed) {
        unsafe {
            let h = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
            if !h.is_null() {
                let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 64);
                if !view.Value.is_null() {
                    let p = (view.Value as *const u8).add(offset) as *const f64;
                    let v = std::ptr::read_unaligned(p);
                    UnmapViewOfFile(view);
                    let khz = v.round() as u32;
                    if khz > 0 && khz != last {
                        last = khz;
                        emit(tx, ctx, RadioEvent::Frequency(khz));
                    }
                }
                CloseHandle(h);
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(not(windows))]
fn shared_memory(_: &Config, tx: &Sender<RadioEvent>, _: &AtomicBool, ctx: &eframe::egui::Context) {
    emit(tx, ctx, RadioEvent::Status("Shared memory is Windows-only".into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_radioinfo() {
        let xml = "<RadioInfo><RadioNr>2</RadioNr><TXFreq>1407400</TXFreq></RadioInfo>";
        assert_eq!(parse_n1mm(xml), Some((2, 14074)));
        assert_eq!(parse_n1mm("<x/>"), None);
    }
}
