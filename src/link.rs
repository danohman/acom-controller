//! I/O worker: owns the transport (serial, remote TCP or simulator), frames and
//! decodes the byte stream, acknowledges amplifier messages and paces writes.

use crate::config::{Config, Connection, PowerPulse};
use crate::protocol::commands;
use crate::protocol::frame::Decoder;
use crate::protocol::messages::{Message, addr, decode};
use crate::sim::{SimControl, Simulator};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Minimum spacing between transmitted messages (spec: 20 ms).
const TX_SPACING: Duration = Duration::from_millis(20);

pub trait Port: Send {
    /// Read available bytes. Ok(0) means "nothing yet" (timeout), not EOF.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
    fn name(&self) -> String;
    /// Drive the modem control lines (used for remote power-on).
    fn set_lines(&mut self, _dtr: bool, _rts: bool) -> io::Result<()> {
        Err(io::Error::other("modem lines not available on this connection"))
    }
}

struct SerialPortImpl {
    port: Box<dyn serialport::SerialPort>,
    name: String,
}

impl Port for SerialPortImpl {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => Ok(0),
            Err(e) => Err(e),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.port.write_all(data)?;
        self.port.flush()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn set_lines(&mut self, dtr: bool, rts: bool) -> io::Result<()> {
        self.port.write_data_terminal_ready(dtr).map_err(io::Error::other)?;
        self.port.write_request_to_send(rts).map_err(io::Error::other)
    }
}

struct TcpPort {
    s: TcpStream,
    name: String,
}

impl Port for TcpPort {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.s.read(buf) {
            Ok(0) => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "connection closed")),
            Ok(n) => Ok(n),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => Ok(0),
            Err(e) => Err(e),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.s.write_all(data)
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}

/// Commands from the UI to the worker.
pub enum Cmd {
    /// A complete wire frame.
    Send(Vec<u8>),
    /// Pulse DTR/RTS to switch the amplifier on.
    PowerPulse(PowerPulse, u32, bool),
    Stop,
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected(String),
    Disconnected(String),
    Message(Message),
    /// Informational line for the communication log.
    Rx { address: u8, len: usize },
    Tx { len: usize },
    Clients(usize),
    /// Short status text for the user.
    Notice(String),
}

pub struct LinkHandle {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
    pub sim: Option<Arc<SimControl>>,
}

impl LinkHandle {
    pub fn send(&self, wire: Vec<u8>) {
        let _ = self.tx.send(Cmd::Send(wire));
    }
}

impl Drop for LinkHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

fn open(cfg: &Config, sim: &Option<Arc<SimControl>>) -> Result<Box<dyn Port>, String> {
    match cfg.connection {
        Connection::Demo => Ok(Box::new(Simulator::new(sim.clone().unwrap_or_default()))),
        Connection::Serial => {
            if cfg.com_port.is_empty() {
                return Err("No COM port selected".into());
            }
            let port = serialport::new(&cfg.com_port, 9600)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .flow_control(serialport::FlowControl::None)
                .timeout(Duration::from_millis(20))
                .open()
                .map_err(|e| format!("{}: {e}", cfg.com_port))?;
            Ok(Box::new(SerialPortImpl { port, name: cfg.com_port.clone() }))
        }
        Connection::Remote => {
            let addr = format!("{}:{}", cfg.remote_host, cfg.remote_port);
            let sa = std::net::ToSocketAddrs::to_socket_addrs(&addr)
                .map_err(|e| format!("{addr}: {e}"))?
                .next()
                .ok_or_else(|| format!("{addr}: no address"))?;
            let s = TcpStream::connect_timeout(&sa, Duration::from_secs(3)).map_err(|e| format!("{addr}: {e}"))?;
            let _ = s.set_nodelay(true);
            let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
            Ok(Box::new(TcpPort { s, name: addr }))
        }
    }
}

/// Start the worker thread for the given configuration.
pub fn start(cfg: &Config, ctx: eframe::egui::Context) -> LinkHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<Cmd>();
    let (ev_tx, ev_rx) = crossbeam_channel::unbounded::<Event>();
    let sim = (cfg.connection == Connection::Demo).then(|| Arc::new(SimControl::default()));
    let cfg = cfg.clone();
    let sim2 = sim.clone();
    std::thread::Builder::new()
        .name("amp-link".into())
        .spawn(move || run(cfg, sim2, cmd_rx, ev_tx, ctx))
        .expect("spawn link thread");
    LinkHandle { tx: cmd_tx, rx: ev_rx, sim }
}

/// Optional TCP server that mirrors the amplifier byte stream to remote clients.
struct Bridge {
    listener: TcpListener,
    clients: Vec<TcpStream>,
}

impl Bridge {
    fn new(port: u16) -> Option<Self> {
        let l = TcpListener::bind(("0.0.0.0", port)).ok()?;
        l.set_nonblocking(true).ok()?;
        Some(Self { listener: l, clients: Vec::new() })
    }

    /// Returns true if the number of clients changed.
    fn accept(&mut self) -> bool {
        let mut changed = false;
        while let Ok((s, _)) = self.listener.accept() {
            let _ = s.set_nodelay(true);
            let _ = s.set_nonblocking(true);
            self.clients.push(s);
            changed = true;
        }
        changed
    }

    fn broadcast(&mut self, data: &[u8]) -> bool {
        let before = self.clients.len();
        self.clients.retain_mut(|c| c.write_all(data).is_ok());
        before != self.clients.len()
    }

    /// Collect bytes sent by clients (to forward to the amplifier).
    fn collect(&mut self) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        let before = self.clients.len();
        let mut buf = [0u8; 256];
        self.clients.retain_mut(|c| match c.read(&mut buf) {
            Ok(0) => false,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                true
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
            Err(_) => false,
        });
        (out, before != self.clients.len())
    }
}

fn run(cfg: Config, sim: Option<Arc<SimControl>>, cmds: Receiver<Cmd>, ev: Sender<Event>, ctx: eframe::egui::Context) {
    let send = |e: Event| {
        let _ = ev.send(e);
        ctx.request_repaint();
    };
    let mut port = match open(&cfg, &sim) {
        Ok(p) => p,
        Err(e) => {
            send(Event::Disconnected(e));
            return;
        }
    };
    send(Event::Connected(port.name()));

    let sharing = cfg.share_enabled && cfg.connection == Connection::Serial;
    let mut bridge = if sharing { Bridge::new(cfg.share_port) } else { None };
    let mut dec = Decoder::new();
    let mut last_tx = Instant::now() - TX_SPACING;
    let mut last_rx = Instant::now();
    let mut buf = [0u8; 512];
    let stop = AtomicBool::new(false);

    // Pace and write a frame.
    let mut write = |port: &mut Box<dyn Port>, wire: &[u8], last_tx: &mut Instant| -> io::Result<()> {
        let since = last_tx.elapsed();
        if since < TX_SPACING {
            std::thread::sleep(TX_SPACING - since);
        }
        port.write_all(wire)?;
        *last_tx = Instant::now();
        Ok(())
    };

    // Ask for everything we want to show statically.
    let mut startup: Vec<Vec<u8>> = vec![commands::telemetry_enable(true)];
    for a in [
        addr::SYSTEM_INFO, addr::SETTINGS, addr::HELLO_1, addr::HELLO_2, addr::ANT_NAMES,
        addr::BAND_INFO, addr::ATU_INFO, addr::ERROR_CODES,
    ] {
        startup.push(commands::request_message(a));
    }
    for w in &startup {
        if let Err(e) = write(&mut port, w, &mut last_tx) {
            send(Event::Disconnected(e.to_string()));
            return;
        }
    }
    let mut last_poke = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        // 1. Commands from the UI
        loop {
            match cmds.try_recv() {
                Ok(Cmd::Send(w)) => {
                    if let Err(e) = write(&mut port, &w, &mut last_tx) {
                        send(Event::Disconnected(e.to_string()));
                        return;
                    }
                    send(Event::Tx { len: w.len() });
                }
                Ok(Cmd::PowerPulse(kind, ms, hold)) => {
                    let (d, r) = match kind {
                        PowerPulse::Dtr => (true, false),
                        PowerPulse::Rts => (false, true),
                        PowerPulse::Both => (true, true),
                    };
                    // Windows asserts DTR/RTS when a port is opened, so drive both low
                    // first to guarantee a clean rising edge on the selected line(s).
                    let _ = port.set_lines(false, false);
                    std::thread::sleep(Duration::from_millis(300));
                    match port.set_lines(d, r) {
                        Ok(()) => {
                            std::thread::sleep(Duration::from_millis(ms as u64));
                            if !hold {
                                let _ = port.set_lines(false, false);
                            }
                            send(Event::Notice(format!("Power-on: DTR={d} RTS={r} asserted {ms} ms{}", if hold { ", left asserted" } else { "" })));
                        }
                        Err(e) => send(Event::Notice(format!("Power-on failed: could not set control lines ({e})"))),
                    }
                }
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }

        // 2. Remote clients -> amplifier
        if let Some(b) = bridge.as_mut() {
            let mut changed = b.accept();
            let (data, c2) = b.collect();
            changed |= c2;
            if !data.is_empty() {
                if let Err(e) = write(&mut port, &data, &mut last_tx) {
                    send(Event::Disconnected(e.to_string()));
                    return;
                }
            }
            if changed {
                send(Event::Clients(b.clients.len()));
            }
        }

        // While the amplifier is silent (off, or just powered on) keep re-sending the
        // startup requests so it is picked up as soon as it boots.
        if last_rx.elapsed() > Duration::from_secs(3) && last_poke.elapsed() > Duration::from_secs(3) {
            last_poke = Instant::now();
            for w in &startup {
                if let Err(e) = write(&mut port, w, &mut last_tx) {
                    send(Event::Disconnected(e.to_string()));
                    return;
                }
            }
        }

        // 3. Amplifier -> decode
        match port.read(&mut buf) {
            Ok(0) => {
                if last_rx.elapsed() > Duration::from_millis(150) {
                    dec.reset();
                }
            }
            Ok(n) => {
                last_rx = Instant::now();
                if let Some(b) = bridge.as_mut() {
                    if b.broadcast(&buf[..n]) {
                        send(Event::Clients(b.clients.len()));
                    }
                }
                for f in dec.push(&buf[..n]) {
                    let msg = decode(&f);
                    // Acknowledge everything the amplifier initiates.
                    let _ = write(&mut port, &commands::ack(f.address), &mut last_tx);
                    send(Event::Rx { address: f.address, len: f.data.len() + 4 });
                    send(Event::Message(msg));
                }
            }
            Err(e) => {
                send(Event::Disconnected(e.to_string()));
                return;
            }
        }
    }
}
