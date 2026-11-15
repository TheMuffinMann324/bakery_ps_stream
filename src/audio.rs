//! Low-latency audio playback. Decoded PCM from the console goes into a
//! lock-free ring buffer that the WASAPI callback drains, with simple linear
//! resampling when the device runs at a different rate than the stream.

use anyhow::Context as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::mpsc;
use std::thread::JoinHandle;

/// Keep at most this much audio queued; anything above is dropped to stay in sync.
const MAX_LATENCY_MS: u32 = 120;

pub struct AudioOutput {
    producer: HeapProd<i16>,
    channels: u16,
    max_queued: usize,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput {
    pub fn new(channels: u16, rate: u32) -> anyhow::Result<Self> {
        anyhow::ensure!(channels > 0 && rate > 0, "invalid audio format");
        let capacity = (rate as usize * channels as usize) / 2; // 500 ms
        let (producer, consumer) = HeapRb::<i16>::new(capacity).split();

        // cpal streams are not guaranteed to be Send, so the stream lives on its own thread.
        let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<()>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("audio-out".into())
            .spawn(move || match build_stream(consumer, channels, rate) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        ready_rx.recv().context("audio thread exited")??;

        Ok(Self {
            producer,
            channels,
            max_queued: (rate * MAX_LATENCY_MS / 1000) as usize * channels as usize,
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    pub fn source_channels(&self) -> u16 {
        self.channels
    }

    /// Queue interleaved samples.
    pub fn push(&mut self, samples: &[i16]) {
        if self.producer.occupied_len() > self.max_queued {
            // Too far behind (e.g. device hiccup); drop this packet to recover latency.
            return;
        }
        self.producer.push_slice(samples);
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn build_stream(
    mut consumer: HeapCons<i16>,
    src_channels: u16,
    src_rate: u32,
) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no audio output device")?;
    let supported = device
        .default_output_config()
        .context("no default output config")?;
    let dst_channels = supported.channels() as usize;
    let dst_rate = supported.sample_rate();
    let format = supported.sample_format();
    let config = supported.config();

    let src_ch = src_channels as usize;
    let step = src_rate as f64 / dst_rate as f64;
    // Resampler state: position between `prev` and `next` source frames.
    let mut pos = 0.0f64;
    let mut prev = vec![0f32; src_ch];
    let mut next = vec![0f32; src_ch];
    let mut frame = vec![0i16; src_ch];
    let mut primed = false;

    let mut fill = move |out: &mut dyn FnMut(usize, f32), frames: usize| {
        for i in 0..frames {
            while pos >= 1.0 || !primed {
                prev.copy_from_slice(&next);
                if consumer.pop_slice(&mut frame) == src_ch {
                    for (n, s) in next.iter_mut().zip(&frame) {
                        *n = *s as f32 / 32768.0;
                    }
                } else {
                    // Underrun: fade to silence instead of clicking.
                    for n in next.iter_mut() {
                        *n *= 0.5;
                    }
                }
                if primed {
                    pos -= 1.0;
                }
                primed = true;
            }
            let t = pos as f32;
            for c in 0..dst_channels {
                let sc = if src_ch == 1 { 0 } else { c.min(src_ch - 1) };
                let v = if c < src_ch || src_ch == 1 {
                    prev[sc] + (next[sc] - prev[sc]) * t
                } else {
                    0.0
                };
                out(i * dst_channels + c, v);
            }
            pos += step;
        }
    };

    let err_fn = |e| crate::remote::log_line(format!("[E] Audio stream error: {e}"));
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / dst_channels;
                fill(&mut |i, v| data[i] = v, frames);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / dst_channels;
                fill(
                    &mut |i, v| data[i] = (v.clamp(-1.0, 1.0) * 32767.0) as i16,
                    frames,
                );
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported output sample format {other:?}"),
    };
    stream.play()?;
    crate::remote::log_line(format!(
        "[I] Audio output: {dst_channels} ch @ {dst_rate} Hz ({format:?})"
    ));
    Ok(stream)
}
