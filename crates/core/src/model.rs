//! Basic-Pitch-Modell (ONNX) mit tract ausführen – fensterweise, damit Fortschritt
//! angezeigt werden kann und die Weboberfläche nicht einfriert.

use anyhow::{anyhow, Result};
use tract_onnx::prelude::*;

pub const SAMPLE_RATE: u32 = 22050;
pub const FFT_HOP: usize = 256;
pub const AUDIO_N_SAMPLES: usize = SAMPLE_RATE as usize * 2 - FFT_HOP;
pub const ANNOTATIONS_FPS: usize = SAMPLE_RATE as usize / FFT_HOP;
pub const ANNOT_N_FRAMES: usize = ANNOTATIONS_FPS * 2;
pub const N_NOTES: usize = 88;
pub const N_CONTOURS: usize = 264;
const N_OVERLAPPING_FRAMES: usize = 30;
const OVERLAP_LEN: usize = N_OVERLAPPING_FRAMES * FFT_HOP;
const HOP_SIZE: usize = AUDIO_N_SAMPLES - OVERLAP_LEN;

static MODEL_BYTES: &[u8] = include_bytes!("../assets/nmp.onnx");

/// Rohausgabe des Modells, Zeilen = Frames (ca. 86 pro Sekunde).
#[derive(Clone, Debug, Default)]
pub struct ModelOutput {
    pub n_frames: usize,
    /// Notenaktivität, `n_frames × 88`
    pub notes: Vec<f32>,
    /// Einsatzwahrscheinlichkeit, `n_frames × 88`
    pub onsets: Vec<f32>,
    /// Tonhöhenverlauf in Drittel-Halbtönen, `n_frames × 264`
    pub contours: Vec<f32>,
}

pub struct Model {
    plan: std::sync::Arc<TypedRunnableModel>,
}

impl Model {
    pub fn load() -> Result<Self> {
        let plan = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(MODEL_BYTES))?
            .with_input_fact(0, f32::fact([1, AUDIO_N_SAMPLES, 1]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Model { plan })
    }

    /// Ein Fenster (AUDIO_N_SAMPLES Samples) auswerten → (notes, onsets, contours), je 172 Frames.
    fn run_window(&self, window: &[f32]) -> Result<[Vec<f32>; 3]> {
        let input: Tensor = tract_ndarray::Array3::from_shape_vec((1, AUDIO_N_SAMPLES, 1), window.to_vec())?.into();
        let out = self.plan.run(tvec!(input.into()))?;
        // Ausgabereihenfolge des ONNX-Graphen: onsets, notes, contours
        let get = |i: usize| -> Result<Vec<f32>> {
            Ok(out.get(i).ok_or_else(|| anyhow!("model output missing"))?.to_plain_array_view::<f32>()?.iter().copied().collect())
        };
        Ok([get(1)?, get(0)?, get(2)?])
    }
}

/// Laufende Transkription; `step()` verarbeitet ein Fenster.
pub struct Transcription {
    padded: Vec<f32>,
    original_len: usize,
    next: usize,
    total: usize,
    out: ModelOutput,
}

impl Transcription {
    /// `audio` muss Mono mit 22 050 Hz sein.
    pub fn new(audio: &[f32]) -> Self {
        let mut padded = vec![0.0; OVERLAP_LEN / 2];
        padded.extend_from_slice(audio);
        let total = padded.len().div_ceil(HOP_SIZE).max(1);
        Transcription { padded, original_len: audio.len(), next: 0, total, out: ModelOutput::default() }
    }

    pub fn progress(&self) -> f32 {
        self.next as f32 / self.total as f32
    }

    pub fn done(&self) -> bool {
        self.next >= self.total
    }

    /// Nächstes Fenster auswerten. Gibt `true` zurück, wenn alles verarbeitet ist.
    pub fn step(&mut self, model: &Model) -> Result<bool> {
        if self.done() {
            return Ok(true);
        }
        let start = self.next * HOP_SIZE;
        let mut window = vec![0.0f32; AUDIO_N_SAMPLES];
        let end = (start + AUDIO_N_SAMPLES).min(self.padded.len());
        if start < end {
            window[..end - start].copy_from_slice(&self.padded[start..end]);
        }
        let [n, o, c] = model.run_window(&window)?;
        // Überlappung an beiden Rändern verwerfen
        let olap = N_OVERLAPPING_FRAMES / 2;
        let keep = ANNOT_N_FRAMES - 2 * olap;
        self.out.notes.extend_from_slice(&n[olap * N_NOTES..(olap + keep) * N_NOTES]);
        self.out.onsets.extend_from_slice(&o[olap * N_NOTES..(olap + keep) * N_NOTES]);
        self.out.contours.extend_from_slice(&c[olap * N_CONTOURS..(olap + keep) * N_CONTOURS]);
        self.next += 1;
        Ok(self.done())
    }

    pub fn finish(mut self) -> ModelOutput {
        let n_orig = (self.original_len as f64 * (ANNOTATIONS_FPS as f64 / SAMPLE_RATE as f64)).floor() as usize;
        let n = n_orig.min(self.out.notes.len() / N_NOTES);
        self.out.notes.truncate(n * N_NOTES);
        self.out.onsets.truncate(n * N_NOTES);
        self.out.contours.truncate(n * N_CONTOURS);
        self.out.n_frames = n;
        self.out
    }
}

/// Bequeme Variante ohne Fortschrittsanzeige.
pub fn run(model: &Model, audio: &[f32], mut progress: impl FnMut(f32)) -> Result<ModelOutput> {
    let mut t = Transcription::new(audio);
    while !t.step(model)? {
        progress(t.progress());
    }
    Ok(t.finish())
}
