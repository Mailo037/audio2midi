//! Aus der Modellausgabe Noten machen (Portierung von basic_pitch/note_creation.py).

use crate::model::{ModelOutput, AUDIO_N_SAMPLES, ANNOT_N_FRAMES, FFT_HOP, N_CONTOURS, N_NOTES, SAMPLE_RATE};

const MIDI_OFFSET: usize = 21;
const MAX_FREQ_IDX: usize = 87;
const ENERGY_TOL: usize = 11;

/// Erkannte Note, Zeiten in Sekunden.
#[derive(Clone, Debug, PartialEq)]
pub struct RawNote {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    /// 0–1
    pub amplitude: f32,
    /// Tonhöhenabweichung je Frame in Drittel-Halbtönen
    pub bends: Option<Vec<i32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetectParams {
    pub onset_threshold: f32,
    pub frame_threshold: f32,
    pub min_note_ms: f32,
    /// 0 = keine Grenze
    pub min_freq: f32,
    pub max_freq: f32,
    pub melodia_trick: bool,
    /// Pitch-Bends auch für überlappende Noten behalten
    pub multiple_pitch_bends: bool,
}

impl Default for DetectParams {
    fn default() -> Self {
        DetectParams {
            onset_threshold: 0.5,
            frame_threshold: 0.3,
            min_note_ms: 127.7,
            min_freq: 0.0,
            max_freq: 0.0,
            melodia_trick: true,
            multiple_pitch_bends: false,
        }
    }
}

fn hz_to_midi(hz: f32) -> f32 {
    12.0 * (hz / 440.0).log2() + 69.0
}

/// Zeitpunkt (s) jedes Modell-Frames – inklusive Basic Pitchs Fenster-Korrektur.
pub fn frame_times(n_frames: usize) -> Vec<f64> {
    let hop = FFT_HOP as f64 / SAMPLE_RATE as f64;
    let window_offset = hop * (ANNOT_N_FRAMES as f64 - AUDIO_N_SAMPLES as f64 / FFT_HOP as f64) + 0.0018;
    (0..n_frames)
        .map(|i| i as f64 * hop - window_offset * (i / ANNOT_N_FRAMES) as f64)
        .collect()
}

pub fn detect_notes(out: &ModelOutput, p: &DetectParams) -> Vec<RawNote> {
    let n = out.n_frames;
    if n < 3 {
        return Vec::new();
    }
    let mut frames = out.notes.clone();
    let mut onsets = out.onsets.clone();
    let at = |t: usize, f: usize| t * N_NOTES + f;

    // Frequenzbereich begrenzen
    if p.max_freq > 0.0 {
        let idx = (hz_to_midi(p.max_freq) - MIDI_OFFSET as f32).round().max(0.0) as usize;
        for t in 0..n {
            for f in idx.min(N_NOTES)..N_NOTES {
                frames[at(t, f)] = 0.0;
                onsets[at(t, f)] = 0.0;
            }
        }
    }
    if p.min_freq > 0.0 {
        let idx = (hz_to_midi(p.min_freq) - MIDI_OFFSET as f32).round().max(0.0) as usize;
        for t in 0..n {
            for f in 0..idx.min(N_NOTES) {
                frames[at(t, f)] = 0.0;
                onsets[at(t, f)] = 0.0;
            }
        }
    }

    // Einsätze zusätzlich aus Anstiegen der Notenaktivität ableiten
    let n_diff = 2;
    let mut frame_diff = vec![0f32; n * N_NOTES];
    for t in 0..n {
        for f in 0..N_NOTES {
            let mut d = f32::INFINITY;
            for k in 1..=n_diff {
                let prev = if t >= k { frames[at(t - k, f)] } else { 0.0 };
                d = d.min(frames[at(t, f)] - prev);
            }
            frame_diff[at(t, f)] = if t < n_diff { 0.0 } else { d.max(0.0) };
        }
    }
    let max_on = onsets.iter().cloned().fold(0f32, f32::max);
    let max_fd = frame_diff.iter().cloned().fold(0f32, f32::max);
    if max_fd > 0.0 {
        for (o, d) in onsets.iter_mut().zip(&frame_diff) {
            *o = o.max(max_on * d / max_fd);
        }
    }

    // Lokale Maxima über die Zeit, rückwärts sortiert
    let mut peaks: Vec<(usize, usize)> = Vec::new();
    for t in 1..n - 1 {
        for f in 0..N_NOTES {
            let v = onsets[at(t, f)];
            if v > onsets[at(t - 1, f)] && v > onsets[at(t + 1, f)] && v >= p.onset_threshold {
                peaks.push((t, f));
            }
        }
    }
    peaks.reverse();

    let min_len = (p.min_note_ms / 1000.0 * (SAMPLE_RATE as f32 / FFT_HOP as f32)).round() as usize;
    let thr = p.frame_threshold;
    let mut energy = frames.clone();
    let mut events: Vec<(usize, usize, usize, f32)> = Vec::new();
    let mean = |frames: &[f32], a: usize, b: usize, f: usize| -> f32 {
        (a..b).map(|t| frames[t * N_NOTES + f]).sum::<f32>() / (b - a).max(1) as f32
    };

    for &(start, f) in &peaks {
        if start >= n - 1 {
            continue;
        }
        let mut i = start + 1;
        let mut k = 0;
        while i < n - 1 && k < ENERGY_TOL {
            if energy[at(i, f)] < thr {
                k += 1;
            } else {
                k = 0;
            }
            i += 1;
        }
        i -= k;
        if i - start <= min_len {
            continue;
        }
        for t in start..i {
            energy[at(t, f)] = 0.0;
            if f < MAX_FREQ_IDX {
                energy[at(t, f + 1)] = 0.0;
            }
            if f > 0 {
                energy[at(t, f - 1)] = 0.0;
            }
        }
        events.push((start, i, f, mean(&frames, start, i, f)));
    }

    if p.melodia_trick {
        // Wie np.argmax-Schleife, aber einmal sortiert: Werte ändern sich nur durch Nullsetzen.
        let mut cand: Vec<usize> = (0..n * N_NOTES).filter(|&i| energy[i] > thr).collect();
        cand.sort_by(|&a, &b| energy[b].partial_cmp(&energy[a]).unwrap().then(a.cmp(&b)));
        for idx in cand {
            if energy[idx] <= thr {
                continue;
            }
            let (i_mid, f) = (idx / N_NOTES, idx % N_NOTES);
            energy[idx] = 0.0;
            let clear = |e: &mut Vec<f32>, t: usize| {
                e[at(t, f)] = 0.0;
                if f < MAX_FREQ_IDX {
                    e[at(t, f + 1)] = 0.0;
                }
                if f > 0 {
                    e[at(t, f - 1)] = 0.0;
                }
            };
            let mut i = i_mid + 1;
            let mut k = 0;
            while i < n - 1 && k < ENERGY_TOL {
                if energy[at(i, f)] < thr {
                    k += 1;
                } else {
                    k = 0;
                }
                clear(&mut energy, i);
                i += 1;
            }
            let i_end = i - 1 - k;
            let mut i = i_mid as isize - 1;
            let mut k = 0;
            while i > 0 && k < ENERGY_TOL {
                if energy[at(i as usize, f)] < thr {
                    k += 1;
                } else {
                    k = 0;
                }
                clear(&mut energy, i as usize);
                i -= 1;
            }
            let i_start = (i + 1 + k as isize) as usize;
            if i_end <= i_start || i_end - i_start <= min_len {
                continue;
            }
            events.push((i_start, i_end, f, mean(&frames, i_start, i_end, f)));
        }
    }

    let times = frame_times(n);
    let mut notes: Vec<RawNote> = events
        .into_iter()
        .map(|(s, e, f, amp)| RawNote {
            start: times[s],
            end: times[e.min(n - 1)],
            pitch: (f + MIDI_OFFSET) as u8,
            amplitude: amp,
            bends: Some(pitch_bends(&out.contours, s, e, (f + MIDI_OFFSET) as i32)),
        })
        .collect();

    if !p.multiple_pitch_bends {
        drop_overlapping_bends(&mut notes);
    } else {
        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    }
    notes
}

fn pitch_bends(contours: &[f32], start: usize, end: usize, pitch: i32) -> Vec<i32> {
    const TOL: i32 = 25;
    let wlen = (TOL * 2 + 1) as usize;
    let gauss: Vec<f32> = (0..wlen).map(|k| (-0.5 * ((k as f32 - TOL as f32) / 5.0).powi(2)).exp()).collect();
    let fi = 3 * (pitch - MIDI_OFFSET as i32);
    let f0 = (fi - TOL).max(0);
    let f1 = (fi + TOL + 1).min(N_CONTOURS as i32);
    let g0 = (TOL - fi).max(0);
    let shift = TOL - (TOL - fi).max(0);
    (start..end)
        .map(|t| {
            let mut best = (0, f32::MIN);
            for (j, fbin) in (f0..f1).enumerate() {
                let v = contours[t * N_CONTOURS + fbin as usize] * gauss[(g0 as usize) + j];
                if v > best.1 {
                    best = (j as i32, v);
                }
            }
            best.0 - shift
        })
        .collect()
}

fn drop_overlapping_bends(notes: &mut [RawNote]) {
    notes.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap()
            .then(a.end.partial_cmp(&b.end).unwrap())
            .then(a.pitch.cmp(&b.pitch))
    });
    for i in 0..notes.len() {
        for j in i + 1..notes.len() {
            if notes[j].start >= notes[i].end {
                break;
            }
            notes[i].bends = None;
            notes[j].bends = None;
        }
    }
}
