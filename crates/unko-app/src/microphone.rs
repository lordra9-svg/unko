//! System microphone capture for the Famicom and Karaoke Studio inputs.
//!
//! The NES microphone is a 1-bit signal: the host only needs to decide whether
//! sound is present. This captures the default input device, follows its peak
//! level with a short hold, and exposes a boolean that the player feeds to
//! `NesCore::set_microphone` / `NesCore::set_bandai_microphone`. It is presentation
//! input, not an analog ADC model; the threshold and hold are documented below
//! and can be tuned without touching the core.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Default peak amplitude that counts as "sound present" (about -34 dBFS).
pub const THRESHOLD: f32 = 0.02;
/// Callbacks the signal stays asserted after the last loud buffer.
const HOLD_CALLBACKS: u32 = 12;

/// A running cpal input stream and its gate state.
pub struct MicrophoneInput {
    _stream: cpal::Stream,
    active: Arc<AtomicBool>,
}

impl MicrophoneInput {
    /// Names of the available input devices, without duplicates.
    #[must_use]
    pub fn devices() -> Vec<String> {
        let host = cpal::default_host();
        let mut names = Vec::new();
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                let name = device.to_string();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Opens an input device and starts capturing.
    ///
    /// An empty or `None` name selects the system default. `threshold` is the
    /// peak amplitude that asserts the microphone bit.
    pub fn new(device: Option<&str>, threshold: f32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = match device.filter(|name| !name.is_empty()) {
            Some(name) => {
                let devices = host
                    .input_devices()
                    .map_err(|err| format!("enumerate microphone inputs: {err}"))?;
                let mut found = None;
                for candidate in devices {
                    if candidate.to_string() == name {
                        found = Some(candidate);
                        break;
                    }
                }
                found.ok_or_else(|| format!("microphone input '{name}' was not found"))?
            }
            None => host
                .default_input_device()
                .ok_or_else(|| "no default microphone input device".to_string())?,
        };
        let supported = device
            .default_input_config()
            .map_err(|err| format!("default input config: {err}"))?;
        let config: cpal::StreamConfig = supported.config();
        let active = Arc::new(AtomicBool::new(false));
        let threshold = threshold.clamp(0.001, 1.0);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, active.clone(), threshold)?,
            SampleFormat::I16 => build::<i16>(&device, &config, active.clone(), threshold)?,
            SampleFormat::U16 => build::<u16>(&device, &config, active.clone(), threshold)?,
            other => return Err(format!("unsupported microphone sample format {other:?}")),
        };
        stream
            .play()
            .map_err(|err| format!("start microphone stream: {err}"))?;
        Ok(Self {
            _stream: stream,
            active,
        })
    }

    /// Whether sound is currently detected.
    #[must_use]
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    active: Arc<AtomicBool>,
    threshold: f32,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut hold = 0u32;
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut peak = 0.0f32;
                for sample in data {
                    let value = f32::from_sample(*sample).abs();
                    if value > peak {
                        peak = value;
                    }
                }
                active.store(apply_gate(peak, threshold, &mut hold), Ordering::Relaxed);
            },
            move |err| eprintln!("microphone stream error: {err}"),
            None,
        )
        .map_err(|err| format!("build microphone stream: {err}"))
}

fn apply_gate(peak: f32, threshold: f32, hold: &mut u32) -> bool {
    if peak >= threshold {
        *hold = HOLD_CALLBACKS;
        true
    } else if *hold > 0 {
        *hold -= 1;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_holds_brief_gaps_and_releases_silence() {
        let mut hold = 0;
        assert!(!apply_gate(0.001, THRESHOLD, &mut hold));
        assert!(apply_gate(0.5, THRESHOLD, &mut hold));
        for _ in 0..HOLD_CALLBACKS {
            assert!(apply_gate(0.0, THRESHOLD, &mut hold));
        }
        assert!(!apply_gate(0.0, THRESHOLD, &mut hold));
    }

    #[test]
    fn gate_uses_the_requested_threshold() {
        let mut hold = 0;
        assert!(!apply_gate(0.01, 0.05, &mut hold));
        assert!(!apply_gate(0.04, 0.05, &mut hold));
        assert!(apply_gate(0.06, 0.05, &mut hold));
    }

    #[test]
    fn a_missing_named_device_is_rejected() {
        let Err(err) = MicrophoneInput::new(Some("unko-no-such-microphone"), THRESHOLD) else {
            panic!("a missing device must not silently fall back");
        };
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn opens_the_default_input_when_available() {
        // CI and headless machines may have no input device; the app treats
        // that as a non-fatal condition, so the test does the same.
        let Ok(input) = MicrophoneInput::new(None, THRESHOLD) else {
            return;
        };
        std::thread::sleep(std::time::Duration::from_millis(30));
        let _ = input.active();
    }
}
