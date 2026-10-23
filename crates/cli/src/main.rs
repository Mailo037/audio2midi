//! Kommandozeile: Link oder Datei → MIDI.
//!
//!   audio2midi lied.mp3
//!   audio2midi "https://soundcloud.com/…" --preset Klavier --quantize 1/16 --split 60
//!   audio2midi *.wav --mono --transpose -2 -o meine_midis

use anyhow::{bail, Result};
use audio2midi_core::download::{self, LinkOptions};
use audio2midi_core::post::{note_name, presets, Key, Quantize, VelocityMode};
use audio2midi_core::{audio, export, DetectParams, Model, PostOptions, PrepOptions};
use clap::Parser;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "audio2midi", version, about = "Audio (Link oder Datei) in MIDI umwandeln")]
struct Args {
    /// Links oder Dateipfade
    #[arg(required = true)]
    quellen: Vec<String>,
    /// Ausgabeordner
    #[arg(short, long, default_value = "ausgabe")]
    out: PathBuf,
    /// Voreinstellung: Standard, Klavier, Gesang, Bass, Gitarre, Flöte
    #[arg(long)]
    preset: Option<String>,
    /// Start in Sekunden
    #[arg(long, default_value_t = 0.0)]
    start: f64,
    /// Ende in Sekunden (0 = bis Schluss)
    #[arg(long, default_value_t = 0.0)]
    end: f64,
    /// Lautstärke nicht normalisieren
    #[arg(long)]
    no_normalize: bool,
    /// Einsatz-Empfindlichkeit 0–1 (höher = weniger Noten)
    #[arg(long)]
    onset: Option<f32>,
    /// Noten-Schwelle 0–1
    #[arg(long)]
    frame: Option<f32>,
    /// Mindestlänge einer Note in ms
    #[arg(long)]
    min_note_ms: Option<f32>,
    #[arg(long)]
    min_freq: Option<f32>,
    #[arg(long)]
    max_freq: Option<f32>,
    /// Melodia-Trick ausschalten
    #[arg(long)]
    no_melodia: bool,
    /// Festes Tempo (sonst automatisch)
    #[arg(long)]
    bpm: Option<f64>,
    /// Aus, 1/4, 1/8, "1/8 Triole", 1/16, "1/16 Triole", 1/32
    #[arg(long, default_value = "Aus")]
    quantize: String,
    /// Quantisierungsstärke in %
    #[arg(long, default_value_t = 100.0)]
    strength: f64,
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    transpose: i32,
    /// Tonart festlegen, z. B. "A-Moll" (sonst automatisch)
    #[arg(long)]
    key: Option<String>,
    /// Falsche Töne auf die Tonleiter korrigieren
    #[arg(long)]
    snap_to_key: bool,
    /// Einstimmig (nur höchster Ton)
    #[arg(long)]
    mono: bool,
    /// In zwei Spuren teilen an dieser MIDI-Note (60 = C4)
    #[arg(long)]
    split: Option<u8>,
    /// General-MIDI-Instrument 0–127
    #[arg(long)]
    program: Option<u8>,
    /// Feste Velocity 1–127
    #[arg(long)]
    fixed_velocity: Option<u8>,
    /// Noten unter dieser Velocity entfernen
    #[arg(long, default_value_t = 0)]
    min_velocity: u8,
    /// Wiederholte Töne verbinden, wenn Lücke kleiner (ms)
    #[arg(long, default_value_t = 0.0)]
    merge_gap: f64,
    /// Pitch-Bends behalten
    #[arg(long)]
    pitch_bends: bool,
    /// cookies.txt für YouTube
    #[arg(long)]
    cookies: Option<PathBuf>,
    /// Cookies aus Browser übernehmen, z. B. firefox, chrome
    #[arg(long)]
    cookies_from_browser: Option<String>,
    /// Kein Vorhör-Audio erzeugen
    #[arg(long)]
    no_preview: bool,
}

fn parse_key(s: &str) -> Result<Key> {
    let s = s.trim().to_lowercase().replace(' ', "-");
    for k in Key::all() {
        if k.name().to_lowercase() == s {
            return Ok(k);
        }
    }
    bail!("Unbekannte Tonart '{s}' (Beispiel: C-Dur, A-Moll, F#-Moll)")
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()[]".contains(c) { c } else { '_' }).collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() { "audio".into() } else { n.chars().take(100).collect() }
}

fn main() -> Result<()> {
    let a = Args::parse();
    let mut detect = DetectParams::default();
    let mut post = PostOptions::default();
    if let Some(name) = &a.preset {
        let p = presets()
            .into_iter()
            .find(|p| p.name.to_lowercase().starts_with(&name.to_lowercase()))
            .ok_or_else(|| anyhow::anyhow!("Unbekannte Voreinstellung '{name}'"))?;
        detect = p.detect;
        post.mono = p.mono;
        post.program = p.program;
    }
    if let Some(v) = a.onset { detect.onset_threshold = v }
    if let Some(v) = a.frame { detect.frame_threshold = v }
    if let Some(v) = a.min_note_ms { detect.min_note_ms = v }
    if let Some(v) = a.min_freq { detect.min_freq = v }
    if let Some(v) = a.max_freq { detect.max_freq = v }
    detect.melodia_trick = !a.no_melodia;
    detect.multiple_pitch_bends = a.pitch_bends;
    if let Some(b) = a.bpm {
        post.auto_tempo = false;
        post.bpm = b;
    }
    post.quantize = Quantize::parse(&a.quantize).ok_or_else(|| anyhow::anyhow!("Unbekanntes Raster '{}'", a.quantize))?;
    post.quantize_strength = a.strength;
    post.transpose = a.transpose;
    post.key = a.key.as_deref().map(parse_key).transpose()?;
    post.snap_to_key = a.snap_to_key;
    post.mono |= a.mono;
    if let Some(s) = a.split {
        post.split_hands = true;
        post.split_point = s;
    }
    if let Some(p) = a.program { post.program = p.min(127) }
    if let Some(v) = a.fixed_velocity {
        post.velocity_mode = VelocityMode::Fixed;
        post.fixed_velocity = v.clamp(1, 127);
    }
    post.min_velocity = a.min_velocity;
    post.merge_gap_ms = a.merge_gap;
    post.keep_pitch_bends = a.pitch_bends;
    let prep = PrepOptions { start: a.start, end: a.end, normalize: !a.no_normalize };
    let link = LinkOptions { cookies_file: a.cookies.clone(), cookies_browser: a.cookies_from_browser.clone() };

    let model = Model::load()?;
    let mut failed = 0;
    for (i, src) in a.quellen.iter().enumerate() {
        println!("[{}/{}] {src}", i + 1, a.quellen.len());
        let res = (|| -> Result<()> {
            let (audio, title) = if download::is_url(src) {
                let d = download::download(src, &link, |p, msg| {
                    print!("\r   {:3.0}%  {msg}   ", p * 100.0);
                    let _ = std::io::stdout().flush();
                })?;
                println!();
                (audio::decode(d.bytes, Some(&d.extension))?, d.title)
            } else {
                let p = Path::new(src);
                (audio::load_file(p)?, p.file_stem().and_then(|s| s.to_str()).unwrap_or("audio").to_string())
            };
            let analysis = audio2midi_core::analyze(&model, &audio, &prep, |p| {
                print!("\r   {:3.0}%  Erkenne Noten …   ", p * 100.0);
                let _ = std::io::stdout().flush();
            })?;
            println!("\r   100%  Erkenne Noten …   ");
            let song = analysis.song(&detect, &post);

            let name = safe_name(&title);
            let dir = a.out.join(&name);
            std::fs::create_dir_all(&dir)?;
            let midi = dir.join(format!("{name}.mid"));
            std::fs::write(&midi, export::midi_bytes(&song))?;
            std::fs::write(dir.join(format!("{name}_noten.csv")), export::csv(&song))?;
            std::fs::write(dir.join(format!("{name}_pianoroll.svg")), export::pianoroll_svg(&song, &title))?;
            let rate = audio2midi_core::model::SAMPLE_RATE;
            std::fs::write(dir.join(format!("{name}_audio.wav")), audio::wav_bytes(&analysis.audio.samples, rate, 1))?;
            if !a.no_preview {
                let synth = export::synthesize(&song, rate);
                std::fs::write(dir.join(format!("{name}_midi_vorschau.wav")), audio::wav_bytes(&synth, rate, 1))?;
                let mix = export::comparison_mix(&analysis.audio.samples, &synth);
                std::fs::write(dir.join(format!("{name}_vergleich.wav")), audio::wav_bytes(&mix, rate, 2))?;
            }

            let range = song.pitch_range().map(|(l, h)| format!("{} – {}", note_name(l), note_name(h))).unwrap_or("–".into());
            println!("   Titel      {title}");
            println!("   Länge      {:.1} s", analysis.audio.duration());
            println!("   Tempo      {} BPM{}", song.bpm, if song.tempo_detected { " (erkannt)" } else { "" });
            println!("   Tonart     {}{}", song.key.name(), if song.key_detected { " (erkannt)" } else { "" });
            println!("   Noten      {}", song.note_count());
            println!("   Spuren     {}", song.tracks.len());
            println!("   Tonumfang  {range}");
            println!("   → {}", midi.display());
            Ok(())
        })();
        if let Err(e) = res {
            failed += 1;
            eprintln!("   FEHLER: {e:#}");
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
