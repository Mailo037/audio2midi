//! MIDI, CSV, SVG-Pianoroll und Vorhör-Synthesizer.

use crate::post::{note_name, Song};
use midly::num::{u14, u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind};

const TPQ: u16 = 480;

/// Standard-MIDI-Datei (Typ 1) erzeugen.
pub fn midi_bytes(song: &Song) -> Vec<u8> {
    let ticks = |sec: f64| -> u64 { (sec * song.bpm / 60.0 * TPQ as f64).round().max(0.0) as u64 };
    let mut tracks: Vec<Vec<TrackEvent<'_>>> = Vec::new();

    let us_per_beat = (60_000_000.0 / song.bpm).round() as u32;
    tracks.push(vec![
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us_per_beat))) },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)) },
        TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) },
    ]);

    for (i, track) in song.tracks.iter().enumerate() {
        // Kanal 10 (Index 9) ist Schlagzeug – überspringen
        let ch = u4::new(if i >= 9 { (i + 1) as u8 % 16 } else { i as u8 });
        // (Tick, Sortierung, Ereignis): Note-Off vor Note-On am selben Tick
        let mut evs: Vec<(u64, u8, TrackEventKind<'_>)> = Vec::new();
        evs.push((0, 0, TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes()))));
        evs.push((0, 1, TrackEventKind::Midi { channel: ch, message: MidiMessage::ProgramChange { program: u7::new(track.program) } }));
        for n in &track.notes {
            let (a, mut b) = (ticks(n.start), ticks(n.end));
            if b <= a {
                b = a + 1;
            }
            evs.push((a, 3, TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOn { key: u7::new(n.pitch), vel: u7::new(n.velocity.max(1)) } }));
            evs.push((b, 2, TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOff { key: u7::new(n.pitch), vel: u7::new(0) } }));
        }
        if !track.bends.is_empty() {
            for (t, v) in &track.bends {
                let raw = (*v as i32 + 8192).clamp(0, 16383) as u16;
                evs.push((ticks(*t), 1, TrackEventKind::Midi { channel: ch, message: MidiMessage::PitchBend { bend: PitchBend(u14::new(raw)) } }));
            }
            let end = track.notes.iter().map(|n| ticks(n.end)).max().unwrap_or(0);
            evs.push((end + 1, 1, TrackEventKind::Midi { channel: ch, message: MidiMessage::PitchBend { bend: PitchBend(u14::new(8192)) } }));
        }
        evs.sort_by_key(|e| (e.0, e.1));
        let mut last = 0u64;
        let mut out: Vec<TrackEvent<'_>> = evs
            .into_iter()
            .map(|(t, _, kind)| {
                let d = t - last;
                last = t;
                TrackEvent { delta: u28::new(d.min(0x0FFF_FFFF) as u32), kind }
            })
            .collect();
        out.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
        tracks.push(out);
    }

    let smf = Smf { header: Header::new(Format::Parallel, Timing::Metrical(u15::new(TPQ))), tracks };
    let mut buf = Vec::new();
    smf.write_std(&mut buf).expect("write MIDI");
    buf
}

pub fn csv(song: &Song) -> String {
    let mut s = String::from("Track;Start (s);End (s);Duration (s);MIDI note;Name;Velocity\n");
    for t in &song.tracks {
        for n in &t.notes {
            s += &format!(
                "{};{:.3};{:.3};{:.3};{};{};{}\n",
                t.name,
                n.start,
                n.end,
                n.end - n.start,
                n.pitch,
                note_name(n.pitch),
                n.velocity
            );
        }
    }
    s
}

pub const TRACK_COLORS: [[u8; 3]; 4] = [[94, 234, 212], [249, 168, 212], [252, 211, 77], [147, 197, 253]];

/// Pianoroll als SVG (für die Kommandozeile).
pub fn pianoroll_svg(song: &Song, title: &str) -> String {
    let (w, h, left, top) = (1400.0, 500.0, 44.0, 28.0);
    let Some((lo, hi)) = song.pitch_range() else {
        return format!("<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}'/>");
    };
    let (lo, hi) = (lo.saturating_sub(2), (hi + 2).min(127));
    let end = song.end_time().max(0.1);
    let rows = (hi - lo + 1) as f64;
    let rh = (h - top - 24.0) / rows;
    let x = |t: f64| left + t / end * (w - left - 10.0);
    let y = |p: u8| top + (hi - p) as f64 * rh;
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' font-family='sans-serif' font-size='11'>\
         <rect width='100%' height='100%' fill='#14161c'/>\
         <text x='{left}' y='18' fill='#e5e7eb' font-size='13'>{}</text>",
        xml_escape(title)
    );
    for p in lo..=hi {
        if matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
            s += &format!("<rect x='{left}' y='{:.1}' width='{:.1}' height='{rh:.1}' fill='#1b1e26'/>", y(p), w - left - 10.0);
        }
        if p % 12 == 0 {
            s += &format!("<text x='4' y='{:.1}' fill='#9aa0ab'>{}</text>", y(p) + rh * 0.8, note_name(p));
        }
    }
    let step = if end > 120.0 { 10 } else if end > 30.0 { 5 } else { 1 };
    let mut t = 0;
    while (t as f64) <= end {
        s += &format!(
            "<line x1='{0:.1}' x2='{0:.1}' y1='{top}' y2='{1:.1}' stroke='#262a34'/><text x='{0:.1}' y='{2:.1}' fill='#9aa0ab'>{t}s</text>",
            x(t as f64),
            h - 24.0,
            h - 8.0
        );
        t += step;
    }
    for (i, tr) in song.tracks.iter().enumerate() {
        let [r, g, b] = TRACK_COLORS[i % TRACK_COLORS.len()];
        for n in &tr.notes {
            s += &format!(
                "<rect x='{:.2}' y='{:.2}' width='{:.2}' height='{:.2}' rx='1.5' fill='rgb({r},{g},{b})' fill-opacity='{:.2}'/>",
                x(n.start),
                y(n.pitch) + rh * 0.1,
                (x(n.end) - x(n.start)).max(1.0),
                rh * 0.8,
                0.35 + 0.65 * n.velocity as f64 / 127.0
            );
        }
    }
    s + "</svg>"
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&apos;")
}

/// Einfacher, klavierähnlicher Klang zum Vorhören.
pub fn synthesize(song: &Song, rate: u32) -> Vec<f32> {
    let sr = rate as f64;
    let len = ((song.end_time() + 1.0) * sr) as usize + 1;
    let mut buf = vec![0f32; len];
    let tail = 0.25;
    for tr in &song.tracks {
        for n in &tr.notes {
            let dur = (n.end - n.start).max(0.05);
            let f = 440.0 * 2f64.powf((n.pitch as f64 - 69.0) / 12.0);
            let decay = 1.5 + f / 600.0;
            let amp = n.velocity as f64 / 127.0 * 0.2;
            let i0 = (n.start * sr) as usize;
            let count = ((dur + tail) * sr) as usize;
            let w = 2.0 * std::f64::consts::PI * f / sr;
            for k in 0..count {
                let idx = i0 + k;
                if idx >= len {
                    break;
                }
                let t = k as f64 / sr;
                let ph = w * k as f64;
                let wave = ph.sin() + 0.4 * (2.0 * ph).sin() + 0.15 * (3.0 * ph).sin();
                let mut env = (-t * decay).exp() * (t / 0.005).min(1.0);
                if t > dur {
                    env *= (1.0 - (t - dur) / tail).max(0.0);
                }
                buf[idx] += (wave * env * amp) as f32;
            }
        }
    }
    let peak = buf.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 0.99 {
        let g = 0.99 / peak;
        buf.iter_mut().for_each(|v| *v *= g);
    }
    buf
}

/// Original (links betont) und MIDI (rechts betont) als Stereo, verschachtelt.
pub fn comparison_mix(original: &[f32], synth: &[f32]) -> Vec<f32> {
    let n = original.len().max(synth.len());
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let a = original.get(i).copied().unwrap_or(0.0) * 0.8;
        let b = synth.get(i).copied().unwrap_or(0.0);
        out.push(a * 0.6 + b * 0.4);
        out.push(b * 0.6 + a * 0.4);
    }
    out
}
