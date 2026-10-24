//! Kommandozeile: Link oder Datei → MIDI.
//!
//!   audio2midi song.mp3
//!   audio2midi "https://soundcloud.com/…" --preset piano --quantize 1/16 --split 60
//!   audio2midi *.wav --mono --transpose -2 -o my_midis

use anyhow::{bail, Result};
use audio2midi_core::download::{self, LinkOptions};
use audio2midi_core::post::{note_name, presets, Key, Quantize, VelocityMode};
use audio2midi_core::{audio, export, DetectParams, Model, PostOptions, PrepOptions};
use clap::Parser;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "audio2midi", version, about = "Convert audio (link or file) to MIDI")]
struct Args {
    /// Links or file paths
    #[arg(required = true)]
    sources: Vec<String>,
    /// Output folder
    #[arg(short, long, default_value = "output")]
    out: PathBuf,
    /// Preset: default, piano, vocals, bass, guitar, whistle
    #[arg(long)]
    preset: Option<String>,
    /// Start in seconds
    #[arg(long, default_value_t = 0.0)]
    start: f64,
    /// End in seconds (0 = until the end)
    #[arg(long, default_value_t = 0.0)]
    end: f64,
    /// Don't normalize the volume
    #[arg(long)]
    no_normalize: bool,
    /// Onset threshold 0–1 (higher = fewer notes)
    #[arg(long)]
    onset: Option<f32>,
    /// Note threshold 0–1
    #[arg(long)]
    frame: Option<f32>,
    /// Minimum note length in ms
    #[arg(long)]
    min_note_ms: Option<f32>,
    #[arg(long)]
    min_freq: Option<f32>,
    #[arg(long)]
    max_freq: Option<f32>,
    /// Turn off the Melodia trick
    #[arg(long)]
    no_melodia: bool,
    /// Fixed tempo (otherwise detected automatically)
    #[arg(long)]
    bpm: Option<f64>,
    /// off, 1/4, 1/8, "1/8 triplet", 1/16, "1/16 triplet", 1/32
    #[arg(long, default_value = "off")]
    quantize: String,
    /// Quantization strength in %
    #[arg(long, default_value_t = 100.0)]
    strength: f64,
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    transpose: i32,
    /// Set the key, e.g. "A minor" (otherwise detected automatically)
    #[arg(long)]
    key: Option<String>,
    /// Snap wrong notes to the scale
    #[arg(long)]
    snap_to_key: bool,
    /// Monophonic (highest note only)
    #[arg(long)]
    mono: bool,
    /// Split into two tracks at this MIDI note (60 = C4)
    #[arg(long)]
    split: Option<u8>,
    /// General MIDI instrument 0–127
    #[arg(long)]
    program: Option<u8>,
    /// Fixed velocity 1–127
    #[arg(long)]
    fixed_velocity: Option<u8>,
    /// Remove notes below this velocity
    #[arg(long, default_value_t = 0)]
    min_velocity: u8,
    /// Merge repeated notes if the gap is smaller than this (ms)
    #[arg(long, default_value_t = 0.0)]
    merge_gap: f64,
    /// Keep pitch bends
    #[arg(long)]
    pitch_bends: bool,
    /// Remove ghost notes (overtones, stray high/low notes), 0–100 %
    #[arg(long, default_value_t = 50.0)]
    cleanup: f64,
    /// cookies.txt for YouTube
    #[arg(long)]
    cookies: Option<PathBuf>,
    /// Use cookies from a browser, e.g. firefox, chrome
    #[arg(long)]
    cookies_from_browser: Option<String>,
    /// Don't create preview audio
    #[arg(long)]
    no_preview: bool,
}

fn parse_key(s: &str) -> Result<Key> {
    let norm = |s: &str| s.trim().to_lowercase().replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    let wanted = norm(s);
    for k in Key::all() {
        if norm(&k.name()) == wanted {
            return Ok(k);
        }
    }
    bail!("Unknown key '{s}' (examples: \"C major\", \"A minor\", \"F# minor\")")
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
            .ok_or_else(|| anyhow::anyhow!("Unknown preset '{name}'"))?;
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
    post.quantize = Quantize::parse(&a.quantize).ok_or_else(|| anyhow::anyhow!("Unknown grid '{}'", a.quantize))?;
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
    post.cleanup = a.cleanup.clamp(0.0, 100.0);
    let prep = PrepOptions { start: a.start, end: a.end, normalize: !a.no_normalize };
    let link = LinkOptions { cookies_file: a.cookies.clone(), cookies_browser: a.cookies_from_browser.clone() };

    let model = Model::load()?;
    let mut failed = 0;
    for (i, src) in a.sources.iter().enumerate() {
        println!("[{}/{}] {src}", i + 1, a.sources.len());
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
                print!("\r   {:3.0}%  Detecting notes …   ", p * 100.0);
                let _ = std::io::stdout().flush();
            })?;
            println!("\r   100%  Detecting notes …   ");
            let song = analysis.song(&detect, &post);

            let name = safe_name(&title);
            let dir = a.out.join(&name);
            std::fs::create_dir_all(&dir)?;
            let midi = dir.join(format!("{name}.mid"));
            std::fs::write(&midi, export::midi_bytes(&song))?;
            std::fs::write(dir.join(format!("{name}_notes.csv")), export::csv(&song))?;
            std::fs::write(dir.join(format!("{name}_pianoroll.svg")), export::pianoroll_svg(&song, &title))?;
            let rate = audio2midi_core::model::SAMPLE_RATE;
            std::fs::write(dir.join(format!("{name}_audio.wav")), audio::wav_bytes(&analysis.audio.samples, rate, 1))?;
            if !a.no_preview {
                let synth = export::synthesize(&song, rate);
                std::fs::write(dir.join(format!("{name}_midi_preview.wav")), audio::wav_bytes(&synth, rate, 1))?;
                let mix = export::comparison_mix(&analysis.audio.samples, &synth);
                std::fs::write(dir.join(format!("{name}_compare.wav")), audio::wav_bytes(&mix, rate, 2))?;
            }

            let range = song.pitch_range().map(|(l, h)| format!("{} – {}", note_name(l), note_name(h))).unwrap_or("–".into());
            println!("   Title      {title}");
            println!("   Length     {:.1} s", analysis.audio.duration());
            println!("   Tempo      {} BPM{}", song.bpm, if song.tempo_detected { " (detected)" } else { "" });
            println!("   Key        {}{}", song.key.name(), if song.key_detected { " (detected)" } else { "" });
            println!("   Notes      {}", song.note_count());
            println!("   Tracks     {}", song.tracks.len());
            println!("   Range      {range}");
            println!("   → {}", midi.display());
            Ok(())
        })();
        if let Err(e) = res {
            failed += 1;
            eprintln!("   ERROR: {e:#}");
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
