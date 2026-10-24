//! Audio → MIDI: Kernbibliothek (läuft nativ und als WebAssembly).
//!
//! Ablauf: [`audio::decode`] → [`prepare`] → [`model::Transcription`] →
//! [`Analysis::song`] (Notenerkennung + Aufräumen + Nachbearbeitung) → [`export`].

pub mod audio;
pub mod clean;
pub mod export;
pub mod model;
pub mod notes;
pub mod post;

pub use audio::Audio;
pub use model::{Model, ModelOutput, Transcription};
pub use notes::{DetectParams, RawNote};
pub use post::{Key, PostOptions, Quantize, Song, VelocityMode};

/// Vorbereitung vor der Analyse.
#[derive(Clone, Debug, PartialEq)]
pub struct PrepOptions {
    pub start: f64,
    /// 0 = bis zum Ende
    pub end: f64,
    pub normalize: bool,
}

impl Default for PrepOptions {
    fn default() -> Self {
        PrepOptions { start: 0.0, end: 0.0, normalize: true }
    }
}

/// Zuschneiden, normalisieren und auf 22 050 Hz umrechnen.
pub fn prepare(audio: &Audio, p: &PrepOptions) -> Audio {
    let mut a = audio.trimmed(p.start, p.end);
    if p.normalize {
        a.normalize();
    }
    a.resampled(model::SAMPLE_RATE)
}

/// Ergebnis eines Modell-Laufs. Alle Einstellungen danach sind sofort neu berechenbar.
pub struct Analysis {
    /// Analysiertes Audio (Mono, 22 050 Hz)
    pub audio: Audio,
    pub output: ModelOutput,
    /// (BPM, erster Schlag in s)
    pub tempo: (f64, f64),
}

impl Analysis {
    pub fn new(audio: Audio, output: ModelOutput) -> Self {
        let tempo = post::detect_tempo(&output);
        Analysis { audio, output, tempo }
    }

    pub fn raw_notes(&self, d: &DetectParams) -> Vec<RawNote> {
        notes::detect_notes(&self.output, d)
    }

    pub fn song_from_raw(&self, raw: &[RawNote], o: &PostOptions) -> Song {
        let cleaned = clean::clean_notes(raw, o.cleanup / 100.0);
        post::postprocess(&cleaned, o, self.tempo, post::detect_key(&cleaned))
    }

    pub fn song(&self, d: &DetectParams, o: &PostOptions) -> Song {
        self.song_from_raw(&self.raw_notes(d), o)
    }
}

/// Komplettlauf ohne Zwischenschritte (z. B. für die Kommandozeile).
pub fn analyze(model: &Model, audio: &Audio, prep: &PrepOptions, progress: impl FnMut(f32)) -> anyhow::Result<Analysis> {
    let a = prepare(audio, prep);
    let out = model::run(model, &a.samples, progress)?;
    Ok(Analysis::new(a, out))
}

#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
pub mod download;
