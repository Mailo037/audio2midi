"""Weboberfläche: Link oder Datei -> MIDI.

Start:  python app.py   (öffnet http://127.0.0.1:7860)
"""

from __future__ import annotations

import argparse
import time
import zipfile
from dataclasses import fields
from pathlib import Path

import gradio as gr

import converter as c

OUTPUT_ROOT = Path(__file__).parent / "ausgabe"
BROWSERS = ["Keine", "chrome", "firefox", "edge", "brave", "safari", "opera", "vivaldi", "chromium"]
HAS_DEMUCS = c.demucs_available()

CSS = """
#title h1 {margin-bottom: 0}
#title p {margin-top: 4px; opacity: .75}
#go {min-height: 52px; font-size: 1.1em}
"""

# Reihenfolge der Options-Felder, wie sie an convert_ui übergeben werden
OPTION_FIELDS = [f.name for f in fields(c.Options)]


def build_items(links: str, files) -> list[tuple[str, bool]]:
    items = [(l.strip(), True) for l in (links or "").splitlines() if l.strip()]
    for f in files or []:
        items.append((f if isinstance(f, str) else f.name, False))
    return items


def convert_ui(links, files, cookies_browser, cookies_file, *values, progress=gr.Progress()):
    items = build_items(links, files)
    if not items:
        raise gr.Error("Bitte einen Link eingeben oder eine Datei hochladen.")

    opts = c.Options(**dict(zip(OPTION_FIELDS, values)))
    opts.program = int(str(opts.program).split(" ")[0])
    for k in ("transpose", "note_low", "note_high", "min_velocity", "fixed_velocity", "split_point"):
        setattr(opts, k, int(getattr(opts, k)))
    if c.STEMS.get(opts.stem) is not None and not HAS_DEMUCS:
        raise gr.Error("Stem-Trennung braucht Demucs: pip install -r requirements-stems.txt")

    cookies_path = cookies_file if isinstance(cookies_file, str) else getattr(cookies_file, "name", None)
    browser = None if cookies_browser in (None, "Keine") else cookies_browser

    stamp = time.strftime("%Y-%m-%d_%H-%M-%S")
    results, errors = [], []
    for i, (src, is_url) in enumerate(items):
        def prog(p, msg, i=i):
            progress((i + p) / len(items), desc=f"[{i + 1}/{len(items)}] {msg}")

        work = OUTPUT_ROOT / f"{stamp}_{i + 1:02d}"
        try:
            results.append(c.convert(src, work, opts, is_url=is_url, cookies_file=cookies_path,
                                     cookies_browser=browser, progress=prog))
        except Exception as exc:
            errors.append(f"**{Path(src).name if not is_url else src}**: {exc}")

    if not results:
        raise gr.Error("\n\n".join(errors) or "Unbekannter Fehler")

    downloads = []
    for r in results:
        downloads += [r.midi_path, r.csv_path, r.source_audio]
    if len(results) > 1:
        zip_path = OUTPUT_ROOT / f"{stamp}_alle_midis.zip"
        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
            for r in results:
                z.write(r.midi_path, r.midi_path.name)
        downloads.insert(0, zip_path)

    md = []
    for r in results:
        rows = "\n".join(f"| {k} | {v} |" for k, v in r.info.items())
        md.append(f"| Eigenschaft | Wert |\n|---|---|\n{rows}")
    if errors:
        md.append("### ⚠️ Fehler\n" + "\n\n".join(errors))
    md.append(f"<sub>Gespeichert in `{OUTPUT_ROOT}`</sub>")

    first = results[0]
    gallery = [(str(r.pianoroll), r.title) for r in results]
    return ("\n\n---\n\n".join(md), [str(p) for p in downloads if p], gallery,
            str(first.preview_audio) if first.preview_audio else None,
            str(first.mix_audio) if first.mix_audio else None,
            str(first.source_audio))


def apply_preset(name):
    p = c.PRESETS[name]
    return (p["onset_threshold"], p["frame_threshold"], p["min_note_ms"], p["min_freq"], p["max_freq"],
            p["mono"], c.GM_INSTRUMENTS[p["program"]])


def build_ui() -> gr.Blocks:
    d = c.Options()
    with gr.Blocks(title="Audio → MIDI") as ui:
        gr.Markdown("# 🎹 Audio → MIDI\nLink einfügen oder Datei hochladen – Noten werden erkannt "
                    "und als MIDI-Datei ausgegeben.", elem_id="title")

        with gr.Row():
            with gr.Column(scale=3):
                with gr.Tabs():
                    with gr.Tab("🔗 Link"):
                        links = gr.Textbox(label="Link(s) – einer pro Zeile",
                                           placeholder="https://www.youtube.com/watch?v=…\nhttps://soundcloud.com/…",
                                           lines=3)
                        with gr.Accordion("Link-Optionen (bei YouTube-Anmeldefehler)", open=False):
                            cookies_browser = gr.Dropdown(BROWSERS, value="Keine",
                                                          label="Cookies aus Browser übernehmen")
                            cookies_file = gr.File(label="… oder cookies.txt hochladen", file_types=[".txt"])
                    with gr.Tab("📁 Datei"):
                        files = gr.File(label="Audio- oder Videodateien (MP3, WAV, FLAC, M4A, OGG, MP4 …)",
                                        file_count="multiple")
            with gr.Column(scale=2):
                preset = gr.Dropdown(list(c.PRESETS), value="Standard", label="Voreinstellung",
                                     info="Setzt passende Werte für die Notenerkennung")
                stem = gr.Dropdown(list(c.STEMS), value=d.stem, label="Welche Spur umwandeln?",
                                   info=("Trennt das Lied vorher mit KI (Demucs)" if HAS_DEMUCS else
                                         "Nicht verfügbar – requirements-stems.txt installieren"),
                                   interactive=HAS_DEMUCS)
                program = gr.Dropdown(c.GM_INSTRUMENTS, value=c.GM_INSTRUMENTS[d.program],
                                      label="MIDI-Instrument")
                go = gr.Button("In MIDI umwandeln", variant="primary", elem_id="go")

        with gr.Accordion("✂️ Zuschnitt & Vorbereitung", open=False):
            with gr.Row():
                start = gr.Number(d.start, label="Start (Sekunden)", minimum=0)
                end = gr.Number(d.end, label="Ende (Sekunden, 0 = bis Schluss)", minimum=0)
                normalize = gr.Checkbox(d.normalize, label="Lautstärke normalisieren")

        with gr.Accordion("🎯 Notenerkennung", open=False):
            with gr.Row():
                onset = gr.Slider(0.05, 0.95, d.onset_threshold, step=0.05, label="Einsatz-Empfindlichkeit",
                                  info="Höher = weniger, sicherere Notenanfänge")
                frame = gr.Slider(0.05, 0.95, d.frame_threshold, step=0.05, label="Noten-Schwelle",
                                  info="Höher = weniger Noten, kürzere Haltedauer")
                min_note = gr.Slider(10, 500, d.min_note_ms, step=1, label="Mindestlänge einer Note (ms)")
            with gr.Row():
                min_freq = gr.Number(d.min_freq, label="Tiefste Frequenz (Hz, 0 = aus)", minimum=0)
                max_freq = gr.Number(d.max_freq, label="Höchste Frequenz (Hz, 0 = aus)", minimum=0)
                melodia = gr.Checkbox(d.melodia_trick, label="Melodia-Trick (Geisternoten reduzieren)")
                multi_bends = gr.Checkbox(d.multiple_pitch_bends, label="Pitch-Bends je Note (mehrere Spuren)")

        with gr.Accordion("⏱️ Tempo & Quantisierung", open=False):
            with gr.Row():
                auto_tempo = gr.Checkbox(d.auto_tempo, label="Tempo automatisch erkennen")
                bpm = gr.Number(d.bpm, label="Tempo (BPM, wenn nicht automatisch)", minimum=20, maximum=300)
            with gr.Row():
                quantize = gr.Dropdown(list(c.QUANTIZE_GRIDS), value=d.quantize, label="Auf Raster quantisieren")
                q_strength = gr.Slider(0, 100, d.quantize_strength, step=5, label="Stärke (%)")
                q_ends = gr.Checkbox(d.quantize_ends, label="Notenenden mitquantisieren")

        with gr.Accordion("🎼 Tonhöhe & Tonart", open=False):
            with gr.Row():
                transpose = gr.Slider(-24, 24, d.transpose, step=1, label="Transponieren (Halbtöne)")
                key = gr.Dropdown(c.key_names(), value=d.key_override, label="Tonart")
                snap = gr.Checkbox(d.snap_to_key, label="Falsche Töne auf Tonleiter korrigieren")
            with gr.Row():
                note_low = gr.Slider(0, 127, d.note_low, step=1, label="Tiefste erlaubte Note (MIDI)")
                note_high = gr.Slider(0, 127, d.note_high, step=1, label="Höchste erlaubte Note (MIDI)")
                mono = gr.Checkbox(d.mono, label="Einstimmig (nur Melodie / höchster Ton)")

        with gr.Accordion("🔊 Dynamik & Aufräumen", open=False):
            with gr.Row():
                vel_mode = gr.Radio(["Original", "Fest", "Skaliert"], value=d.velocity_mode, label="Anschlagstärke")
                fixed_vel = gr.Slider(1, 127, d.fixed_velocity, step=1, label="Feste Velocity")
                vel_scale = gr.Slider(10, 200, d.velocity_scale, step=5, label="Velocity skalieren (%)")
            with gr.Row():
                min_vel = gr.Slider(0, 127, d.min_velocity, step=1, label="Leise Noten entfernen (unter Velocity)")
                merge_gap = gr.Slider(0, 200, d.merge_gap_ms, step=5,
                                      label="Wiederholte Töne verbinden (Lücke bis ms)")

        with gr.Accordion("💾 MIDI-Ausgabe", open=False):
            with gr.Row():
                split = gr.Checkbox(d.split_hands, label="In zwei Spuren teilen (rechte/linke Hand)")
                split_point = gr.Slider(21, 108, d.split_point, step=1, label="Teilungspunkt (60 = C4)")
                keep_bends = gr.Checkbox(d.keep_pitch_bends, label="Pitch-Bends behalten")
            with gr.Row():
                preview = gr.Checkbox(d.render_preview, label="Vorhör-Audio erzeugen")
                audio_fmt = gr.Radio(["mp3", "wav", "flac"], value=d.audio_format,
                                     label="Format der extrahierten Audiospur")

        gr.Markdown("## Ergebnis")
        with gr.Row():
            with gr.Column(scale=2):
                info = gr.Markdown()
                out_files = gr.File(label="Downloads", file_count="multiple")
            with gr.Column(scale=3):
                roll = gr.Gallery(label="Pianoroll", columns=1, height=460, object_fit="contain", allow_preview=True)
        with gr.Row():
            a_midi = gr.Audio(label="MIDI anhören", type="filepath")
            a_mix = gr.Audio(label="Vergleich: Original + MIDI", type="filepath")
            a_src = gr.Audio(label="Verwendete Audiospur", type="filepath")

        option_inputs = {
            "start": start, "end": end, "normalize": normalize, "stem": stem,
            "onset_threshold": onset, "frame_threshold": frame, "min_note_ms": min_note,
            "min_freq": min_freq, "max_freq": max_freq, "multiple_pitch_bends": multi_bends,
            "melodia_trick": melodia, "auto_tempo": auto_tempo, "bpm": bpm, "quantize": quantize,
            "quantize_strength": q_strength, "quantize_ends": q_ends, "transpose": transpose,
            "snap_to_key": snap, "key_override": key, "mono": mono, "keep_pitch_bends": keep_bends,
            "note_low": note_low, "note_high": note_high, "min_velocity": min_vel,
            "velocity_mode": vel_mode, "fixed_velocity": fixed_vel, "velocity_scale": vel_scale,
            "merge_gap_ms": merge_gap, "program": program, "split_hands": split,
            "split_point": split_point, "render_preview": preview, "audio_format": audio_fmt,
        }
        assert set(option_inputs) == set(OPTION_FIELDS), set(OPTION_FIELDS) ^ set(option_inputs)

        preset.change(apply_preset, preset, [onset, frame, min_note, min_freq, max_freq, mono, program])
        go.click(convert_ui,
                 [links, files, cookies_browser, cookies_file, *[option_inputs[f] for f in OPTION_FIELDS]],
                 [info, out_files, roll, a_midi, a_mix, a_src])
    return ui


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="Audio → MIDI Weboberfläche")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=7860)
    ap.add_argument("--share", action="store_true", help="Öffentlichen Gradio-Link erzeugen")
    ap.add_argument("--no-browser", action="store_true")
    a = ap.parse_args()
    OUTPUT_ROOT.mkdir(exist_ok=True)
    build_ui().queue().launch(server_name=a.host, server_port=a.port, share=a.share,
                              inbrowser=not a.no_browser, css=CSS, allowed_paths=[str(OUTPUT_ROOT)])
