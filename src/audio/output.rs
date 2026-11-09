//! The audio device: one `cpal` output stream whose callback owns the
//! mixer. The game thread talks to it only through a channel of
//! [`Command`]s, so the callback never waits on a lock the game holds.

use std::sync::mpsc::{Receiver, Sender};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::mixer::{Command, Mixer};

pub struct Output {
    // Dropping the stream stops the sound; it lives as long as `Output`.
    _stream: cpal::Stream,
    commands: Sender<Command>,
}

impl Output {
    /// Opens the default output device. `Err` (no device, as on a headless
    /// machine, or a format it refuses) leaves the game silent, never
    /// stopped.
    pub fn open() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let (commands, inbox) = std::sync::mpsc::channel();
        let mixer = Mixer::new(config.sample_rate);
        let channels = usize::from(config.channels);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, mixer, inbox, channels),
            SampleFormat::I16 => build::<i16>(&device, &config, mixer, inbox, channels),
            SampleFormat::U16 => build::<u16>(&device, &config, mixer, inbox, channels),
            SampleFormat::I32 => build::<i32>(&device, &config, mixer, inbox, channels),
            other => return Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            commands,
        })
    }

    pub fn send(&self, command: Command) {
        // A closed channel means the stream died; the game goes on silent.
        let _ = self.commands.send(command);
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut mixer: Mixer,
    inbox: Receiver<Command>,
    channels: usize,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                while let Ok(command) = inbox.try_recv() {
                    mixer.apply(command);
                }
                // Grows once to the device's buffer size, then never again.
                scratch.resize(data.len(), 0.0);
                mixer.render(&mut scratch, channels);
                for (out, &s) in data.iter_mut().zip(&scratch) {
                    *out = T::from_sample(s);
                }
            },
            |err| eprintln!("audio: stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
