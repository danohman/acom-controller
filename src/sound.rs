//! Audible alerts. Tones are synthesised, so no audio assets are embedded.
//! Playback runs on its own thread; if no audio device exists everything is a silent no-op.

use crossbeam_channel::{Sender, unbounded};
use rodio::Source;
use rodio::source::SineWave;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// Amplifier reported a warning-level alarm.
    Warning,
    /// Soft or hard fault.
    Fault,
    /// SWR above the configured threshold while transmitting.
    Swr,
    /// Communication with the amplifier was lost.
    LinkLost,
    /// Communication with the amplifier came back.
    LinkRestored,
    /// Settings-page preview.
    Test,
}

impl Alert {
    /// (frequency Hz, duration ms) tone sequence; 0 Hz is a pause.
    fn tones(self) -> &'static [(f32, u64)] {
        match self {
            Alert::Warning => &[(880.0, 160), (0.0, 80), (880.0, 160)],
            Alert::Fault => &[(1200.0, 200), (800.0, 200), (1200.0, 200), (800.0, 200)],
            Alert::Swr => &[(1000.0, 90), (0.0, 60), (1000.0, 90), (0.0, 60), (1000.0, 90)],
            Alert::LinkLost => &[(660.0, 180), (440.0, 320)],
            Alert::LinkRestored => &[(440.0, 140), (660.0, 220)],
            Alert::Test => &[(660.0, 150), (880.0, 150)],
        }
    }
}

pub struct Sound {
    tx: Sender<(Alert, f32)>,
}

impl Sound {
    pub fn new() -> Self {
        let (tx, rx) = unbounded::<(Alert, f32)>();
        std::thread::spawn(move || {
            // Opened lazily so a machine without audio doesn't cost anything at startup.
            let mut sink: Option<rodio::MixerDeviceSink> = None;
            while let Ok((alert, volume)) = rx.recv() {
                if sink.is_none() {
                    sink = rodio::DeviceSinkBuilder::open_default_sink().ok();
                }
                let Some(s) = &sink else { continue };
                for &(freq, ms) in alert.tones() {
                    let d = Duration::from_millis(ms);
                    if freq > 0.0 {
                        s.mixer().add(SineWave::new(freq).amplify(volume.clamp(0.0, 1.0) * 0.5).take_duration(d));
                    }
                    std::thread::sleep(d);
                }
            }
        });
        Self { tx }
    }

    pub fn play(&self, alert: Alert, volume: f32) {
        let _ = self.tx.send((alert, volume));
    }
}
