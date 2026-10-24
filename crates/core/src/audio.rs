//! Audio laden, mischen, zuschneiden, normalisieren und umrechnen.

use anyhow::{anyhow, bail, Result};
use std::io::Cursor;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// Mono-Audio mit Abtastrate.
#[derive(Clone, Debug, Default)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Audio {
    pub fn duration(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples.len() as f64 / self.rate as f64
        }
    }

    /// Ausschnitt in Sekunden; `end <= start` bedeutet „bis zum Ende“.
    pub fn trimmed(&self, start: f64, end: f64) -> Audio {
        let n = self.samples.len();
        let a = ((start.max(0.0) * self.rate as f64) as usize).min(n);
        let b = if end > start { ((end * self.rate as f64) as usize).clamp(a, n) } else { n };
        Audio { samples: self.samples[a..b].to_vec(), rate: self.rate }
    }

    /// Lautheit angleichen: RMS auf ca. -16 dBFS, Spitzen begrenzt.
    pub fn normalize(&mut self) {
        if self.samples.is_empty() {
            return;
        }
        let rms = (self.samples.iter().map(|s| s * s).sum::<f32>() / self.samples.len() as f32).sqrt();
        let peak = self.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        if rms < 1e-6 || peak < 1e-6 {
            return;
        }
        let gain = (0.158 / rms).min(0.89 / peak);
        for s in &mut self.samples {
            *s *= gain;
        }
    }

    /// Umrechnen auf eine andere Abtastrate (gefensterter Sinc-Interpolator).
    pub fn resampled(&self, rate: u32) -> Audio {
        if rate == self.rate || self.samples.is_empty() {
            return Audio { samples: self.samples.clone(), rate };
        }
        let ratio = rate as f64 / self.rate as f64;
        let out_len = (self.samples.len() as f64 * ratio).round() as usize;
        // Grenzfrequenz bei Herunterrechnen absenken (Anti-Aliasing)
        let cutoff = ratio.min(1.0) * 0.95;
        const HALF: isize = 16;
        let width = (HALF as f64 / cutoff).ceil() as isize;
        let src = &self.samples;
        let n = src.len() as isize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 / ratio;
            let center = pos.floor() as isize;
            let mut acc = 0.0f64;
            for j in (center - width + 1)..=(center + width) {
                if j < 0 || j >= n {
                    continue;
                }
                let x = pos - j as f64;
                let w = x / width as f64;
                if w.abs() >= 1.0 {
                    continue;
                }
                let window = 0.5 + 0.5 * (std::f64::consts::PI * w).cos();
                let arg = std::f64::consts::PI * x * cutoff;
                let sinc = if arg.abs() < 1e-9 { 1.0 } else { arg.sin() / arg };
                acc += src[j as usize] as f64 * sinc * cutoff * window;
            }
            out.push(acc as f32);
        }
        Audio { samples: out, rate }
    }
}

/// Dekodiert eine Audio- oder Videodatei (MP3, WAV, FLAC, OGG, M4A/AAC, MP4, MKV …) zu Mono.
pub fn decode(bytes: Vec<u8>, extension_hint: Option<&str>) -> Result<Audio> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension_hint {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| anyhow!("File format not supported ({e})"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("The file contains no audio track"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("Audio track has no codec information"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| {
            anyhow!("Audio codec not supported ({e}). Please convert Opus/WebM to MP3 or WAV first.")
        })?;

    let mut rate = params.sample_rate.unwrap_or(0);
    let mut mono: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(anyhow!("Read error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) => continue, // beschädigtes Paket überspringen
            Err(e) => return Err(anyhow!("Decoding error: {e}")),
        };
        rate = buf.spec().rate();
        let ch = buf.spec().channels().count().max(1);
        interleaved.clear();
        buf.copy_to_vec_interleaved(&mut interleaved);
        mono.extend(interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
    }
    if mono.is_empty() || rate == 0 {
        bail!("No audio data found");
    }
    Ok(Audio { samples: mono, rate })
}

/// Datei lesen und dekodieren.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(path: &std::path::Path) -> Result<Audio> {
    use anyhow::Context;
    let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());
    decode(bytes, ext.as_deref())
}

/// Mono-WAV (16 Bit) in Bytes schreiben.
pub fn wav_bytes(samples: &[f32], rate: u32, channels: u16) -> Vec<u8> {
    let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).expect("WAV header");
        for s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).expect("write WAV");
        }
        w.finalize().expect("finish WAV");
    }
    cur.into_inner()
}
