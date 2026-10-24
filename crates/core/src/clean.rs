//! Geisternoten entfernen: zerteilte Töne, Obertöne, Untertöne, Ausreißer weit über/unter dem
//! Geschehen und kurze, schwache Schnipsel. Läuft auf den Rohnoten, also ohne neuen Modell-Lauf.

use crate::notes::RawNote;

/// Intervalle (Halbtöne) der ersten Obertöne über dem Grundton: Oktave, Oktave+Quinte, 2 Oktaven …
const OVERTONES: [i32; 6] = [12, 19, 24, 28, 31, 36];
/// Zeitfenster (s) für den musikalischen Zusammenhang einer Note
const CONTEXT: f64 = 3.0;

fn overlap(a: &RawNote, b: &RawNote) -> f64 {
    (a.end.min(b.end) - a.start.max(b.start)).max(0.0)
}

fn dur(n: &RawNote) -> f64 {
    (n.end - n.start).max(1e-3)
}

/// Gewicht einer Note: lange und laute Noten zählen mehr.
fn weight(n: &RawNote) -> f64 {
    dur(n) * n.amplitude as f64
}

/// Gewichtetes Quantil der Tonhöhen, `items` = (Tonhöhe, Gewicht), wird sortiert.
fn quantile(items: &mut [(f64, f64)], q: f64) -> f64 {
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total: f64 = items.iter().map(|i| i.1).sum();
    let mut acc = 0.0;
    for &(v, w) in items.iter() {
        acc += w;
        if acc >= total * q {
            return v;
        }
    }
    items.last().map(|i| i.0).unwrap_or(0.0)
}

/// Indizes der Noten, die zeitlich in `[t0, t1]` liegen (Noten nach Start sortiert, `max_dur` = längste Note).
fn around<'a>(notes: &'a [RawNote], keep: &'a [bool], max_dur: f64, t0: f64, t1: f64) -> impl Iterator<Item = usize> + 'a {
    let first = notes.partition_point(|n| n.start < t0 - max_dur);
    (first..notes.len()).take_while(move |&j| notes[j].start <= t1).filter(move |&j| keep[j] && notes[j].end >= t0)
}

/// Gehaltene Töne, die ein Schlagzeugtreffer o. Ä. zerteilt hat, wieder zusammenfügen:
/// gleiche Tonhöhe, nahtlos anschließend und nicht lauter als der Teil davor.
fn rejoin(mut notes: Vec<RawNote>, ratio: f64) -> Vec<RawNote> {
    notes.sort_by(|a, b| a.pitch.cmp(&b.pitch).then(a.start.partial_cmp(&b.start).unwrap()));
    let mut out: Vec<RawNote> = Vec::with_capacity(notes.len());
    for n in notes {
        if let Some(prev) = out.last_mut() {
            let gap = n.start - prev.end;
            if prev.pitch == n.pitch && gap.abs() < 0.012 && (n.amplitude as f64) < prev.amplitude as f64 * ratio {
                let (d0, d1) = (dur(prev), dur(&n));
                prev.amplitude = ((prev.amplitude as f64 * d0 + n.amplitude as f64 * d1) / (d0 + d1)) as f32;
                prev.end = prev.end.max(n.end);
                prev.bends = match (prev.bends.take(), n.bends) {
                    (Some(mut a), Some(b)) => {
                        a.extend(b);
                        Some(a)
                    }
                    _ => None,
                };
                continue;
            }
        }
        out.push(n);
    }
    out
}

/// Geisternoten entfernen. `strength` 0–1: 0 = nichts ändern, 1 = sehr gründlich.
pub fn clean_notes(raw: &[RawNote], strength: f64) -> Vec<RawNote> {
    let s = strength.clamp(0.0, 1.0);
    if s <= 0.0 || raw.len() < 2 {
        return raw.to_vec();
    }
    let mut notes = rejoin(raw.to_vec(), 0.75 + 0.3 * s);
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(a.pitch.cmp(&b.pitch)));
    let n = notes.len();
    let max_dur = notes.iter().map(dur).fold(0.0, f64::max);
    let mut keep = vec![true; n];

    // 1. Ober- und Untertöne: gleichzeitig klingende, deutlich leisere Note im Obertonabstand
    let over_ratio = 0.55 + 0.45 * s;
    let under_ratio = 0.3 + 0.4 * s;
    let ghost: Vec<bool> = (0..n)
        .map(|i| {
            let b = &notes[i];
            around(&notes, &keep, max_dur, b.start, b.end).any(|j| {
                let a = &notes[j];
                if j == i {
                    return false;
                }
                let d = b.pitch as i32 - a.pitch as i32;
                let shared = overlap(a, b) / dur(b);
                let (amp_a, amp_b) = (a.amplitude as f64, b.amplitude as f64);
                (OVERTONES.contains(&d) && shared >= 0.5 && amp_b < amp_a * over_ratio)
                    || (d == -12 && shared >= 0.8 && b.start >= a.start - 0.03 && amp_b < amp_a * under_ratio)
            })
        })
        .collect();
    for (k, g) in keep.iter_mut().zip(&ghost) {
        *k &= !g;
    }

    // 2. Ausreißer: weit über oder unter allem, was ringsum gespielt wird, und nicht besonders kräftig
    let margin = 12.0 - 7.0 * s;
    let strong = 0.5 + 2.0 * s;
    let outlier: Vec<bool> = (0..n)
        .map(|i| {
            if !keep[i] {
                return false;
            }
            let b = &notes[i];
            let mid = (b.start + b.end) / 2.0;
            let mut ctx: Vec<(f64, f64)> = around(&notes, &keep, max_dur, mid - CONTEXT, mid + CONTEXT)
                .filter(|&j| j != i)
                .map(|j| (notes[j].pitch as f64, weight(&notes[j])))
                .collect();
            if ctx.len() < 3 {
                return false;
            }
            let lo = quantile(&mut ctx, 0.1);
            let hi = quantile(&mut ctx, 0.9);
            let p = b.pitch as f64;
            if p <= hi + margin && p >= lo - margin {
                return false;
            }
            let mut ws: Vec<(f64, f64)> = ctx.iter().map(|c| (c.1, 1.0)).collect();
            weight(b) < quantile(&mut ws, 0.5) * strong
        })
        .collect();

    // 3. Kurze, schwache Schnipsel und kurze Einzelnoten ohne Nachbarn
    let short = 0.1 + 0.15 * s;
    let weak = 0.3 + 0.3 * s;
    let stray: Vec<bool> = (0..n)
        .map(|i| {
            if !keep[i] || outlier[i] {
                return false;
            }
            let b = &notes[i];
            if dur(b) >= short {
                return false;
            }
            let mut near = around(&notes, &keep, max_dur, b.start - 1.0, b.end + 1.0).filter(|&j| j != i).peekable();
            if near.peek().is_none() {
                return true;
            }
            let loudest = near.map(|j| notes[j].amplitude).fold(b.amplitude, f32::max) as f64;
            (b.amplitude as f64) < loudest * weak
        })
        .collect();

    notes
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i] && !outlier[*i] && !stray[*i])
        .map(|(_, note)| note)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, end: f64, pitch: u8, amplitude: f32) -> RawNote {
        RawNote { start, end, pitch, amplitude, bends: None }
    }

    #[test]
    fn removes_overtone_ghost() {
        let raw = vec![note(0.0, 1.0, 60, 0.8), note(0.02, 0.9, 72, 0.4), note(1.0, 2.0, 62, 0.8)];
        let out = clean_notes(&raw, 0.5);
        assert_eq!(out.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![60, 62]);
    }

    #[test]
    fn keeps_strong_octave() {
        let raw = vec![note(0.0, 1.0, 48, 0.8), note(0.0, 1.0, 60, 0.8)];
        assert_eq!(clean_notes(&raw, 0.5).len(), 2);
    }

    #[test]
    fn removes_far_outlier() {
        let mut raw: Vec<RawNote> = (0..8).map(|i| note(i as f64 * 0.5, i as f64 * 0.5 + 0.45, 60 + (i % 5) as u8, 0.7)).collect();
        raw.push(note(1.6, 1.75, 96, 0.5));
        raw.push(note(2.6, 2.75, 30, 0.5));
        let out = clean_notes(&raw, 0.5);
        assert_eq!(out.len(), 8);
        assert!(out.iter().all(|n| (60..65).contains(&n.pitch)));
    }

    #[test]
    fn keeps_strong_high_note() {
        let mut raw: Vec<RawNote> = (0..8).map(|i| note(i as f64 * 0.5, i as f64 * 0.5 + 0.3, 60, 0.5)).collect();
        raw.push(note(4.0, 6.0, 84, 0.9));
        assert_eq!(clean_notes(&raw, 0.5).len(), 9);
    }

    #[test]
    fn rejoins_split_note() {
        let raw = vec![note(0.0, 0.2, 76, 0.9), note(0.2, 0.4, 76, 0.6), note(0.4, 0.6, 76, 0.9)];
        let out = clean_notes(&raw, 0.5);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].start, out[0].end), (0.0, 0.4));
    }

    #[test]
    fn strength_zero_changes_nothing() {
        let raw = vec![note(0.0, 1.0, 60, 0.8), note(0.0, 1.0, 72, 0.3)];
        assert_eq!(clean_notes(&raw, 0.0), raw);
    }
}
