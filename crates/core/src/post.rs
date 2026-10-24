//! Nachbearbeitung, Tempo- und Tonarterkennung.

use crate::model::{ModelOutput, N_NOTES};
use crate::notes::{frame_times, DetectParams, RawNote};

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub fn note_name(pitch: u8) -> String {
    format!("{}{}", NOTE_NAMES[pitch as usize % 12], pitch as i32 / 12 - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub root: u8,
    pub minor: bool,
}

impl Key {
    pub fn name(&self) -> String {
        format!("{} {}", NOTE_NAMES[self.root as usize], if self.minor { "minor" } else { "major" })
    }
    pub fn all() -> Vec<Key> {
        (0..2).flat_map(|m| (0..12).map(move |r| Key { root: r, minor: m == 1 })).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantize {
    Off,
    Quarter,
    Eighth,
    EighthTriplet,
    Sixteenth,
    SixteenthTriplet,
    ThirtySecond,
}

impl Quantize {
    pub const ALL: [Quantize; 7] = [
        Quantize::Off,
        Quantize::Quarter,
        Quantize::Eighth,
        Quantize::EighthTriplet,
        Quantize::Sixteenth,
        Quantize::SixteenthTriplet,
        Quantize::ThirtySecond,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Quantize::Off => "Off",
            Quantize::Quarter => "1/4",
            Quantize::Eighth => "1/8",
            Quantize::EighthTriplet => "1/8 triplet",
            Quantize::Sixteenth => "1/16",
            Quantize::SixteenthTriplet => "1/16 triplet",
            Quantize::ThirtySecond => "1/32",
        }
    }
    /// Rasterweite in Schlägen
    pub fn beats(&self) -> Option<f64> {
        match self {
            Quantize::Off => None,
            Quantize::Quarter => Some(1.0),
            Quantize::Eighth => Some(0.5),
            Quantize::EighthTriplet => Some(1.0 / 3.0),
            Quantize::Sixteenth => Some(0.25),
            Quantize::SixteenthTriplet => Some(1.0 / 6.0),
            Quantize::ThirtySecond => Some(0.125),
        }
    }
    pub fn parse(s: &str) -> Option<Quantize> {
        Quantize::ALL.into_iter().find(|q| q.label().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelocityMode {
    Original,
    Fixed,
    Scaled,
}

/// Alle Einstellungen nach der Notenerkennung. Änderungen hier brauchen kein neues Modell-Lauf.
#[derive(Clone, Debug, PartialEq)]
pub struct PostOptions {
    pub auto_tempo: bool,
    pub bpm: f64,
    pub quantize: Quantize,
    /// 0–100 %
    pub quantize_strength: f64,
    pub quantize_ends: bool,
    pub transpose: i32,
    /// `None` = automatisch erkennen
    pub key: Option<Key>,
    pub snap_to_key: bool,
    pub mono: bool,
    pub keep_pitch_bends: bool,
    /// Geisternoten entfernen (Obertöne, Ausreißer), 0–100 %
    pub cleanup: f64,
    pub note_low: u8,
    pub note_high: u8,
    pub min_velocity: u8,
    pub velocity_mode: VelocityMode,
    pub fixed_velocity: u8,
    /// Prozent
    pub velocity_scale: f64,
    pub merge_gap_ms: f64,
    pub program: u8,
    pub split_hands: bool,
    pub split_point: u8,
}

impl Default for PostOptions {
    fn default() -> Self {
        PostOptions {
            auto_tempo: true,
            bpm: 120.0,
            quantize: Quantize::Off,
            quantize_strength: 100.0,
            quantize_ends: true,
            transpose: 0,
            key: None,
            snap_to_key: false,
            mono: false,
            keep_pitch_bends: false,
            cleanup: 50.0,
            note_low: 0,
            note_high: 127,
            min_velocity: 0,
            velocity_mode: VelocityMode::Original,
            fixed_velocity: 100,
            velocity_scale: 100.0,
            merge_gap_ms: 0.0,
            program: 0,
            split_hands: false,
            split_point: 60,
        }
    }
}

pub struct Preset {
    pub name: &'static str,
    pub detect: DetectParams,
    pub mono: bool,
    pub program: u8,
}

pub fn presets() -> Vec<Preset> {
    let d = |on: f32, fr: f32, ms: f32, lo: f32, hi: f32| DetectParams {
        onset_threshold: on,
        frame_threshold: fr,
        min_note_ms: ms,
        min_freq: lo,
        max_freq: hi,
        ..DetectParams::default()
    };
    vec![
        Preset { name: "Default", detect: DetectParams::default(), mono: false, program: 0 },
        Preset { name: "Piano", detect: d(0.55, 0.3, 80.0, 27.5, 4200.0), mono: false, program: 0 },
        Preset { name: "Vocals / melody (monophonic)", detect: d(0.6, 0.4, 120.0, 80.0, 1100.0), mono: true, program: 53 },
        Preset { name: "Bass", detect: d(0.5, 0.35, 100.0, 30.0, 400.0), mono: true, program: 33 },
        Preset { name: "Guitar", detect: d(0.5, 0.3, 90.0, 80.0, 1400.0), mono: false, program: 25 },
        Preset { name: "Whistle / flute", detect: d(0.6, 0.4, 120.0, 250.0, 3000.0), mono: true, program: 73 },
    ]
}

/// Bearbeitete Note
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: u8,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub name: String,
    pub program: u8,
    pub notes: Vec<Note>,
    /// (Zeit in s, Wert −8192…8191) bei ±2 Halbtönen Bend-Bereich
    pub bends: Vec<(f64, i16)>,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub bpm: f64,
    pub tempo_detected: bool,
    pub key: Key,
    pub key_detected: bool,
    pub tracks: Vec<Track>,
}

impl Song {
    pub fn end_time(&self) -> f64 {
        self.tracks.iter().flat_map(|t| t.notes.iter().map(|n| n.end)).fold(0.0, f64::max)
    }
    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.notes.len()).sum()
    }
    pub fn pitch_range(&self) -> Option<(u8, u8)> {
        let it = self.tracks.iter().flat_map(|t| t.notes.iter().map(|n| n.pitch));
        let lo = it.clone().min()?;
        Some((lo, it.max()?))
    }
}

// ---------------------------------------------------------------------------
// Tonart (Krumhansl-Schmuckler)

const MAJOR_PROFILE: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR_PROFILE: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const MAJOR_SCALE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR_SCALE: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];

fn correlation(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    if da == 0.0 || db == 0.0 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

pub fn detect_key(notes: &[RawNote]) -> Option<Key> {
    let mut hist = [0f64; 12];
    for n in notes {
        hist[n.pitch as usize % 12] += (n.end - n.start) * n.amplitude as f64;
    }
    if hist.iter().sum::<f64>() == 0.0 {
        return None;
    }
    let mut best = (f64::MIN, Key { root: 0, minor: false });
    for key in Key::all() {
        let profile = if key.minor { &MINOR_PROFILE } else { &MAJOR_PROFILE };
        let mut rolled = [0f64; 12];
        for i in 0..12 {
            rolled[(i + key.root as usize) % 12] = profile[i];
        }
        let r = correlation(&hist, &rolled);
        if r > best.0 {
            best = (r, key);
        }
    }
    Some(best.1)
}

fn snap_pitch(pitch: i32, key: Key) -> i32 {
    let scale = if key.minor { MINOR_SCALE } else { MAJOR_SCALE };
    let in_scale = |p: i32| scale.iter().any(|s| (key.root as i32 + *s as i32) % 12 == p.rem_euclid(12));
    if in_scale(pitch) {
        return pitch;
    }
    for d in [-1, 1, -2, 2] {
        if in_scale(pitch + d) {
            return pitch + d;
        }
    }
    pitch
}

// ---------------------------------------------------------------------------
// Tempo aus der Einsatz-Hüllkurve des Modells

/// Gibt (BPM, Zeitpunkt des ersten Schlags) zurück.
pub fn detect_tempo(out: &ModelOutput) -> (f64, f64) {
    let n = out.n_frames;
    if n < 200 {
        return (120.0, 0.0);
    }
    let times = frame_times(n);
    let fps = (n - 1) as f64 / (times[n - 1] - times[0]).max(1e-6);
    let mut env: Vec<f64> = (0..n)
        .map(|t| out.onsets[t * N_NOTES..(t + 1) * N_NOTES].iter().map(|v| *v as f64).sum())
        .collect();
    // Mittelwert abziehen und positiv halten
    let mean = env.iter().sum::<f64>() / n as f64;
    for v in &mut env {
        *v = (*v - mean).max(0.0);
    }
    let sample = |x: f64| -> f64 {
        let i = x.floor() as usize;
        if i + 1 >= n {
            return 0.0;
        }
        let f = x - i as f64;
        env[i] * (1.0 - f) + env[i + 1] * f
    };
    let mut best = (f64::MIN, 120.0);
    let mut bpm = 60.0;
    while bpm <= 200.0 {
        let lag = 60.0 * fps / bpm;
        let mut acc = 0.0;
        // Grundschlag + doppelte Periode stärken die richtige Metrik
        for t in 0..n {
            acc += env[t] * (sample(t as f64 + lag) + 0.5 * sample(t as f64 + 2.0 * lag));
        }
        acc /= (n as f64 - lag).max(1.0);
        // Vorliebe für ~120 BPM (log-normal, 1 Oktave Breite) wie librosa
        let prior = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let score = acc * prior;
        if score > best.0 {
            best = (score, bpm);
        }
        bpm += 0.5;
    }
    let bpm = best.1;
    let period = 60.0 * fps / bpm;
    let mut best_phase = (f64::MIN, 0.0);
    let steps = period.ceil() as usize;
    for p in 0..steps {
        let mut acc = 0.0;
        let mut x = p as f64;
        while x < n as f64 {
            acc += sample(x);
            x += period;
        }
        if acc > best_phase.0 {
            best_phase = (acc, p as f64);
        }
    }
    let anchor = best_phase.1 / fps + times[0];
    ((bpm * 10.0).round() / 10.0, anchor)
}

// ---------------------------------------------------------------------------
// Nachbearbeitung

fn make_mono(mut notes: Vec<Note>) -> Vec<Note> {
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(b.pitch.cmp(&a.pitch)));
    let mut out: Vec<Note> = Vec::new();
    for n in notes {
        if let Some(prev) = out.last_mut() {
            if n.start < prev.end - 1e-4 {
                if n.start - prev.start < 0.03 {
                    // praktisch gleichzeitig: höhere Note gewinnt
                    if n.pitch > prev.pitch {
                        *prev = n;
                    }
                    continue;
                }
                prev.end = n.start;
            }
        }
        out.push(n);
    }
    out.retain(|n| n.end - n.start > 0.01);
    out
}

fn merge_repeats(mut notes: Vec<Note>, gap: f64) -> Vec<Note> {
    notes.sort_by(|a, b| a.pitch.cmp(&b.pitch).then(a.start.partial_cmp(&b.start).unwrap()));
    let mut out: Vec<Note> = Vec::new();
    for n in notes {
        if let Some(prev) = out.last_mut() {
            if prev.pitch == n.pitch && n.start - prev.end <= gap {
                prev.end = prev.end.max(n.end);
                prev.velocity = prev.velocity.max(n.velocity);
                continue;
            }
        }
        out.push(n);
    }
    out
}

fn quantize(notes: Vec<Note>, grid: f64, anchor: f64, strength: f64, ends: bool) -> Vec<Note> {
    let q = |t: f64| {
        let target = anchor + ((t - anchor) / grid).round() * grid;
        (t + (target - t) * strength).max(0.0)
    };
    let mut out: Vec<Note> = notes
        .into_iter()
        .map(|mut n| {
            let dur = n.end - n.start;
            n.start = q(n.start);
            n.end = if ends { q(n.end) } else { n.start + dur };
            if n.end - n.start < grid * 0.5 * strength || n.end <= n.start {
                n.end = n.start + (grid * strength).max(dur * (1.0 - strength)).max(0.02);
            }
            n
        })
        .collect();
    // doppelte Noten nach dem Quantisieren entfernen
    out.sort_by(|a, b| {
        a.start.partial_cmp(&b.start).unwrap().then(a.pitch.cmp(&b.pitch)).then(b.velocity.cmp(&a.velocity))
    });
    out.dedup_by(|b, a| a.pitch == b.pitch && (a.start - b.start).abs() < 1e-4);
    out
}

fn bend_value(bins: i32) -> i16 {
    (bins as f64 * 4096.0 / 3.0).round().clamp(-8192.0, 8191.0) as i16
}

/// Rohe Noten mit den Optionen in Spuren umwandeln.
pub fn postprocess(raw: &[RawNote], o: &PostOptions, tempo: (f64, f64), detected_key: Option<Key>) -> Song {
    let (bpm, anchor) = if o.auto_tempo { tempo } else { (o.bpm, 0.0) };
    let key = o.key.or(detected_key).unwrap_or(Key { root: 0, minor: false });
    let grid = o.quantize.beats().map(|b| b * 60.0 / bpm);
    let strength = o.quantize_strength / 100.0;

    // Gruppen: mit Pitch-Bends eine Spur je Tonhöhe, wenn Bends überlappen dürfen (wie Basic Pitch)
    let keep_bends = o.keep_pitch_bends && !o.split_hands && grid.is_none();
    let mut groups: Vec<Vec<&RawNote>> = Vec::new();
    if keep_bends && raw.iter().any(|n| n.bends.is_some()) && has_overlapping_bends(raw) {
        let mut by_pitch: std::collections::BTreeMap<u8, Vec<&RawNote>> = Default::default();
        for n in raw {
            by_pitch.entry(n.pitch).or_default().push(n);
        }
        groups.extend(by_pitch.into_values());
    } else {
        groups.push(raw.iter().collect());
    }

    let mut tracks = Vec::new();
    for group in groups {
        let mut bends = Vec::new();
        let mut notes: Vec<Note> = Vec::new();
        for r in group {
            let vel = (r.amplitude * 127.0).round().clamp(1.0, 127.0) as u8;
            if vel < o.min_velocity {
                continue;
            }
            let mut p = (r.pitch as i32 + o.transpose).clamp(0, 127);
            if o.snap_to_key {
                let k = Key { root: ((key.root as i32 + o.transpose).rem_euclid(12)) as u8, minor: key.minor };
                p = snap_pitch(p, k).clamp(0, 127);
            }
            let p = p as u8;
            if p < o.note_low || p > o.note_high {
                continue;
            }
            if keep_bends {
                if let Some(b) = &r.bends {
                    let len = b.len().max(1) as f64;
                    for (i, v) in b.iter().enumerate() {
                        bends.push((r.start + (r.end - r.start) * i as f64 / (len - 1.0).max(1.0), bend_value(*v)));
                    }
                }
            }
            notes.push(Note { start: r.start, end: r.end, pitch: p, velocity: vel });
        }
        if o.merge_gap_ms > 0.0 {
            notes = merge_repeats(notes, o.merge_gap_ms / 1000.0);
        }
        if o.mono {
            notes = make_mono(notes);
        }
        if let Some(g) = grid {
            notes = quantize(notes, g, anchor, strength, o.quantize_ends);
        }
        for n in &mut notes {
            n.velocity = match o.velocity_mode {
                VelocityMode::Original => n.velocity,
                VelocityMode::Fixed => o.fixed_velocity.max(1),
                VelocityMode::Scaled => (n.velocity as f64 * o.velocity_scale / 100.0).round().clamp(1.0, 127.0) as u8,
            };
        }
        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(a.pitch.cmp(&b.pitch)));
        bends.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        if o.split_hands {
            let (hi, lo): (Vec<Note>, Vec<Note>) = notes.into_iter().partition(|n| n.pitch >= o.split_point);
            for (name, ns) in [("Right hand / high", hi), ("Left hand / low", lo)] {
                if !ns.is_empty() {
                    tracks.push(Track { name: name.into(), program: o.program, notes: ns, bends: Vec::new() });
                }
            }
        } else if !notes.is_empty() {
            tracks.push(Track { name: GM_NAMES[o.program as usize].into(), program: o.program, notes, bends });
        }
    }
    Song {
        bpm,
        tempo_detected: o.auto_tempo,
        key,
        key_detected: o.key.is_none() && detected_key.is_some(),
        tracks,
    }
}

fn has_overlapping_bends(raw: &[RawNote]) -> bool {
    let mut with: Vec<&RawNote> = raw.iter().filter(|n| n.bends.is_some()).collect();
    with.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    with.windows(2).any(|w| w[1].start < w[0].end)
}

pub const GM_NAMES: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano",
    "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavinet", "Celesta", "Glockenspiel",
    "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer", "Drawbar Organ",
    "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica",
    "Tango Accordion", "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)",
    "Electric Guitar (clean)", "Electric Guitar (muted)", "Overdriven Guitar", "Distortion Guitar",
    "Guitar Harmonics", "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)",
    "Fretless Bass", "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2", "Violin", "Viola",
    "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2", "Choir Aahs",
    "Voice Oohs", "Synth Voice", "Orchestra Hit", "Trumpet", "Trombone", "Tuba", "Muted Trumpet",
    "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2", "Soprano Sax", "Alto Sax",
    "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet", "Piccolo", "Flute",
    "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina", "Lead 1 (square)",
    "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)", "Lead 5 (charang)", "Lead 6 (voice)",
    "Lead 7 (fifths)", "Lead 8 (bass + lead)", "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)",
    "Pad 4 (choir)", "Pad 5 (bowed)", "Pad 6 (metallic)", "Pad 7 (halo)", "Pad 8 (sweep)",
    "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)", "FX 5 (brightness)",
    "FX 6 (goblins)", "FX 7 (echoes)", "FX 8 (sci-fi)", "Sitar", "Banjo", "Shamisen", "Koto",
    "Kalimba", "Bagpipe", "Fiddle", "Shanai", "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock",
    "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal", "Guitar Fret Noise", "Breath Noise",
    "Seashore", "Bird Tweet", "Telephone Ring", "Helicopter", "Applause", "Gunshot",
];
