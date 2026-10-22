"""Kommandozeile: Link oder Datei -> MIDI.

Beispiele:
    python cli.py lied.mp3
    python cli.py "https://www.youtube.com/watch?v=…" --stem Gesang --mono --quantize 1/16
    python cli.py *.wav --preset Klavier --transpose -2 -o meine_midis
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import converter as c


def main() -> int:
    ap = argparse.ArgumentParser(description="Audio (Link oder Datei) in MIDI umwandeln")
    ap.add_argument("quellen", nargs="+", help="Links oder Dateipfade")
    ap.add_argument("-o", "--out", default="ausgabe", help="Ausgabeordner (Standard: ausgabe)")
    ap.add_argument("--preset", choices=list(c.PRESETS), help="Voreinstellung")
    ap.add_argument("--stem", choices=list(c.STEMS), default=c.Options.stem)
    ap.add_argument("--start", type=float, default=0.0)
    ap.add_argument("--end", type=float, default=0.0)
    ap.add_argument("--onset", type=float, help="Einsatz-Empfindlichkeit 0–1")
    ap.add_argument("--frame", type=float, help="Noten-Schwelle 0–1")
    ap.add_argument("--min-note-ms", type=float)
    ap.add_argument("--min-freq", type=float)
    ap.add_argument("--max-freq", type=float)
    ap.add_argument("--bpm", type=float, help="Festes Tempo (sonst automatisch)")
    ap.add_argument("--quantize", choices=list(c.QUANTIZE_GRIDS), default="Aus")
    ap.add_argument("--transpose", type=int, default=0)
    ap.add_argument("--snap-to-key", action="store_true")
    ap.add_argument("--mono", action="store_true")
    ap.add_argument("--split", type=int, metavar="NOTE", help="In zwei Spuren teilen an MIDI-Note")
    ap.add_argument("--program", type=int, help="GM-Instrument 0–127")
    ap.add_argument("--fixed-velocity", type=int)
    ap.add_argument("--cookies", help="cookies.txt für YouTube")
    ap.add_argument("--cookies-from-browser", help="z. B. chrome, firefox")
    ap.add_argument("--no-preview", action="store_true")
    a = ap.parse_args()

    opts = c.Options(stem=a.stem, start=a.start, end=a.end, quantize=a.quantize,
                     transpose=a.transpose, snap_to_key=a.snap_to_key, render_preview=not a.no_preview)
    if a.preset:
        p = c.PRESETS[a.preset]
        opts.onset_threshold, opts.frame_threshold = p["onset_threshold"], p["frame_threshold"]
        opts.min_note_ms, opts.min_freq, opts.max_freq = p["min_note_ms"], p["min_freq"], p["max_freq"]
        opts.mono, opts.program = p["mono"], p["program"]
    for attr, val in (("onset_threshold", a.onset), ("frame_threshold", a.frame), ("min_note_ms", a.min_note_ms),
                      ("min_freq", a.min_freq), ("max_freq", a.max_freq), ("program", a.program)):
        if val is not None:
            setattr(opts, attr, val)
    if a.mono:
        opts.mono = True
    if a.bpm:
        opts.auto_tempo, opts.bpm = False, a.bpm
    if a.split is not None:
        opts.split_hands, opts.split_point = True, a.split
    if a.fixed_velocity:
        opts.velocity_mode, opts.fixed_velocity = "Fest", a.fixed_velocity

    out_root = Path(a.out)
    failed = 0
    for i, src in enumerate(a.quellen, 1):
        is_url = src.startswith(("http://", "https://"))
        print(f"[{i}/{len(a.quellen)}] {src}")
        try:
            r = c.convert(src, out_root / f"{i:02d}", opts, is_url=is_url, cookies_file=a.cookies,
                          cookies_browser=a.cookies_from_browser,
                          progress=lambda p, m: print(f"   {int(p * 100):3d}%  {m}", flush=True))
        except Exception as exc:
            failed += 1
            print(f"   FEHLER: {exc}", file=sys.stderr)
            continue
        for k, v in r.info.items():
            print(f"   {k:10s} {v}")
        print(f"   → {r.midi_path}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
