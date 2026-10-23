//! Vergleich mit der Python-Referenz von Basic Pitch.
//! Aufruf: cargo run --release -p audio2midi-core --example compare -- <basis-pfad ohne Endung>

use audio2midi_core::{model, notes, DetectParams, ModelOutput};

fn read_f32(path: &str) -> Vec<f32> {
    std::fs::read(path).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Länge unterschiedlich");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn load_ref_notes(path: &str) -> Vec<(f64, f64, u8)> {
    let s = std::fs::read_to_string(path).unwrap();
    let mut out = Vec::new();
    for item in s.trim().trim_start_matches('[').trim_end_matches(']').split("], [") {
        let v: Vec<&str> = item.trim_matches(|c| c == '[' || c == ']').split(", ").collect();
        if v.len() >= 3 {
            out.push((v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].parse().unwrap()));
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

fn main() {
    let base = std::env::args().nth(1).expect("Basis-Pfad");
    let audio = read_f32(&format!("{base}.f32"));
    let m = model::Model::load().unwrap();
    let t = std::time::Instant::now();
    let out = model::run(&m, &audio, |_| {}).unwrap();
    println!("Modell: {} Frames in {:?}", out.n_frames, t.elapsed());

    let reference = ModelOutput {
        notes: read_f32(&format!("{base}.note")),
        onsets: read_f32(&format!("{base}.onset")),
        contours: read_f32(&format!("{base}.contour")),
        n_frames: read_f32(&format!("{base}.note")).len() / 88,
    };
    println!("Frames Rust/Python: {}/{}", out.n_frames, reference.n_frames);
    println!(
        "max. Abweichung  notes {:.2e}  onsets {:.2e}  contours {:.2e}",
        max_diff(&out.notes, &reference.notes),
        max_diff(&out.onsets, &reference.onsets),
        max_diff(&out.contours, &reference.contours)
    );

    for (mel, file) in [(false, "notes"), (true, "notes_mel")] {
        let p = DetectParams { melodia_trick: mel, ..DetectParams::default() };
        let expected = load_ref_notes(&format!("{base}.{file}.json"));
        for (label, src) in [("Python-Modellausgabe", &reference), ("Rust-Modellausgabe", &out)] {
            let mut got: Vec<(f64, f64, u8)> = notes::detect_notes(src, &p).iter().map(|n| (n.start, n.end, n.pitch)).collect();
            got.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let same = got.len() == expected.len()
                && got.iter().zip(&expected).all(|(a, b)| a.2 == b.2 && (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3);
            println!("melodia={mel:5} {label}: {} Noten (Python {}) → {}", got.len(), expected.len(), if same { "IDENTISCH" } else { "ABWEICHUNG" });
            if !same {
                for (a, b) in got.iter().zip(&expected).take(5) {
                    println!("   rust {a:?}  py {b:?}");
                }
            }
        }
    }
}
