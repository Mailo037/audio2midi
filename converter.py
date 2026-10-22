"""Kernlogik: Audio aus Link/Datei holen und in MIDI umwandeln.

Pipeline:
    Quelle (Link oder Datei)
      -> Audio extrahieren / zuschneiden (ffmpeg)
      -> optional Stem-Trennung (Demucs)
      -> Transkription (Spotify Basic Pitch, ONNX)
      -> Nachbearbeitung (Quantisierung, Transponieren, Tonart, Monophonie, ...)
      -> MIDI, Vorhör-Audio, Pianoroll, CSV
"""

from __future__ import annotations

import csv
import hashlib
import logging
import re
import shutil
import subprocess
import warnings
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional

import numpy as np

warnings.filterwarnings("ignore", category=UserWarning)
logging.getLogger().setLevel(logging.ERROR)

import pretty_midi  # noqa: E402
import soundfile as sf  # noqa: E402

ProgressFn = Callable[[float, str], None]

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "H"]

# Krumhansl-Kessler-Profile für die Tonartbestimmung
_MAJOR_PROFILE = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
_MINOR_PROFILE = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])
_MAJOR_SCALE = [0, 2, 4, 5, 7, 9, 11]
_MINOR_SCALE = [0, 2, 3, 5, 7, 8, 10]

STEMS = {
    "Alles (keine Trennung)": None,
    "Gesang": ("vocals", False),
    "Ohne Gesang (Instrumental)": ("vocals", True),
    "Bass": ("bass", False),
    "Andere Instrumente (Klavier, Gitarre, Synth …)": ("other", False),
    "Ohne Schlagzeug": ("drums", True),
}

QUANTIZE_GRIDS = {
    "Aus": None,
    "1/4": 1.0,
    "1/8": 0.5,
    "1/8 Triole": 1 / 3,
    "1/16": 0.25,
    "1/16 Triole": 1 / 6,
    "1/32": 0.125,
}

# Voreinstellungen für typische Quellen
PRESETS = {
    "Standard": dict(onset_threshold=0.5, frame_threshold=0.3, min_note_ms=127.7,
                     min_freq=0, max_freq=0, mono=False, program=0),
    "Klavier": dict(onset_threshold=0.55, frame_threshold=0.3, min_note_ms=80,
                    min_freq=27.5, max_freq=4200, mono=False, program=0),
    "Gesang / Melodie (einstimmig)": dict(onset_threshold=0.6, frame_threshold=0.4, min_note_ms=120,
                                          min_freq=80, max_freq=1100, mono=True, program=53),
    "Bass": dict(onset_threshold=0.5, frame_threshold=0.35, min_note_ms=100,
                 min_freq=30, max_freq=400, mono=True, program=33),
    "Gitarre": dict(onset_threshold=0.5, frame_threshold=0.3, min_note_ms=90,
                    min_freq=80, max_freq=1400, mono=False, program=25),
    "Pfeifen / Flöte": dict(onset_threshold=0.6, frame_threshold=0.4, min_note_ms=120,
                            min_freq=250, max_freq=3000, mono=True, program=73),
}

GM_INSTRUMENTS = [f"{i:03d} {pretty_midi.program_to_instrument_name(i)}" for i in range(128)]


@dataclass
class Options:
    # Zuschnitt
    start: float = 0.0
    end: float = 0.0  # 0 = bis zum Ende
    normalize: bool = True
    # Stem-Trennung
    stem: str = "Alles (keine Trennung)"
    # Basic Pitch
    onset_threshold: float = 0.5
    frame_threshold: float = 0.3
    min_note_ms: float = 127.7
    min_freq: float = 0.0  # 0 = keine Grenze
    max_freq: float = 0.0
    multiple_pitch_bends: bool = False
    melodia_trick: bool = True
    # Tempo
    auto_tempo: bool = True
    bpm: float = 120.0
    # Nachbearbeitung
    quantize: str = "Aus"
    quantize_strength: float = 100.0  # Prozent
    quantize_ends: bool = True
    transpose: int = 0
    snap_to_key: bool = False
    key_override: str = "Automatisch"
    mono: bool = False
    keep_pitch_bends: bool = False
    note_low: int = 0
    note_high: int = 127
    min_velocity: int = 0
    velocity_mode: str = "Original"  # Original | Fest | Skaliert
    fixed_velocity: int = 100
    velocity_scale: float = 100.0  # Prozent
    merge_gap_ms: float = 0.0  # gleiche Tonhöhe zusammenfügen, wenn Lücke kleiner
    program: int = 0
    split_hands: bool = False
    split_point: int = 60
    # Ausgaben
    render_preview: bool = True
    audio_format: str = "mp3"


@dataclass
class Result:
    title: str
    midi_path: Path
    source_audio: Path
    preview_audio: Optional[Path] = None
    mix_audio: Optional[Path] = None
    pianoroll: Optional[Path] = None
    csv_path: Optional[Path] = None
    info: dict = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Hilfsfunktionen
# ---------------------------------------------------------------------------

def _noop(_: float, __: str) -> None:
    pass


def safe_name(name: str, fallback: str = "audio") -> str:
    name = re.sub(r"[^\w\-. ()\[\]äöüÄÖÜß]+", "_", name, flags=re.UNICODE).strip(" ._")
    return (name or fallback)[:120]


def note_name(pitch: int) -> str:
    return f"{NOTE_NAMES[pitch % 12]}{pitch // 12 - 1}"


def key_names() -> list[str]:
    return ["Automatisch"] + [f"{n}-Dur" for n in NOTE_NAMES] + [f"{n}-Moll" for n in NOTE_NAMES]


def _run_ffmpeg(args: list[str]) -> None:
    if not shutil.which("ffmpeg"):
        raise RuntimeError("ffmpeg wurde nicht gefunden. Bitte installieren (siehe README).")
    proc = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"ffmpeg-Fehler: {proc.stderr.strip()[-500:]}")


# ---------------------------------------------------------------------------
# 1. Quelle
# ---------------------------------------------------------------------------

def download_audio(url: str, out_dir: Path, cookies_file: Optional[str] = None,
                   cookies_browser: Optional[str] = None, progress: ProgressFn = _noop) -> tuple[Path, str]:
    """Lädt die Audiospur eines Links (YouTube, SoundCloud, Bandcamp, Vimeo, direkte MP3 …)."""
    import yt_dlp

    out_dir.mkdir(parents=True, exist_ok=True)

    def hook(d):
        if d.get("status") == "downloading":
            total = d.get("total_bytes") or d.get("total_bytes_estimate") or 0
            if total:
                progress(0.05 + 0.15 * d.get("downloaded_bytes", 0) / total, "Lade Audio herunter …")

    opts = {
        "format": "bestaudio/best",
        "outtmpl": str(out_dir / "download.%(ext)s"),
        "noplaylist": True,
        "quiet": True,
        "no_warnings": True,
        "noprogress": True,
        "progress_hooks": [hook],
        "postprocessors": [{"key": "FFmpegExtractAudio", "preferredcodec": "wav"}],
    }
    if cookies_file:
        opts["cookiefile"] = cookies_file
    if cookies_browser:
        opts["cookiesfrombrowser"] = (cookies_browser,)

    try:
        with yt_dlp.YoutubeDL(opts) as ydl:
            info = ydl.extract_info(url, download=True)
    except Exception as exc:  # yt-dlp wirft viele verschiedene Fehlertypen
        msg = str(exc)
        if "Sign in to confirm" in msg or "bot" in msg.lower():
            msg += ("\n\nTipp: YouTube verlangt eine Anmeldung. Wähle unter 'Link-Optionen' deinen Browser "
                    "für Cookies aus oder lade eine cookies.txt hoch.")
        raise RuntimeError(f"Download fehlgeschlagen: {msg}") from exc

    files = sorted(out_dir.glob("download.*"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not files:
        raise RuntimeError("Download fehlgeschlagen: keine Audiodatei erhalten.")
    return files[0], info.get("title") or "download"


def prepare_audio(src: Path, out_dir: Path, opts: Options) -> Path:
    """Wandelt beliebiges Audio/Video in WAV (44,1 kHz, Stereo) um und schneidet zu."""
    dst = out_dir / "input.wav"
    args = []
    if opts.start > 0:
        args += ["-ss", f"{opts.start:.3f}"]
    args += ["-i", str(src)]
    if opts.end and opts.end > opts.start:
        args += ["-t", f"{opts.end - opts.start:.3f}"]
    filters = ["loudnorm=I=-16:TP=-1.5:LRA=11"] if opts.normalize else []
    if filters:
        args += ["-af", ",".join(filters)]
    args += ["-vn", "-ac", "2", "-ar", "44100", str(dst)]
    _run_ffmpeg(args)
    if not dst.exists() or dst.stat().st_size < 1000:
        raise RuntimeError("Die Datei enthält keine (lesbare) Audiospur.")
    return dst


# ---------------------------------------------------------------------------
# 2. Stem-Trennung
# ---------------------------------------------------------------------------

_DEMUCS_MODEL = None


def demucs_available() -> bool:
    try:
        import demucs  # noqa: F401
        import torch  # noqa: F401
        return True
    except ImportError:
        return False


def separate_stem(wav: Path, out_dir: Path, stem_label: str, progress: ProgressFn = _noop) -> Path:
    spec = STEMS.get(stem_label)
    if spec is None:
        return wav
    if not demucs_available():
        raise RuntimeError("Für die Stem-Trennung wird Demucs benötigt: pip install -r requirements-stems.txt")

    import torch
    from demucs.apply import apply_model
    from demucs.pretrained import get_model

    global _DEMUCS_MODEL
    progress(0.25, "Lade Demucs-Modell (beim ersten Mal ~80 MB Download) …")
    if _DEMUCS_MODEL is None:
        _DEMUCS_MODEL = get_model("htdemucs")
        _DEMUCS_MODEL.eval()
    model = _DEMUCS_MODEL

    audio, sr = sf.read(str(wav), dtype="float32", always_2d=True)  # (samples, ch)
    mix = torch.from_numpy(audio.T.copy())
    ref = mix.mean(0)
    mix = (mix - ref.mean()) / (ref.std() + 1e-8)

    progress(0.3, "Trenne Spuren (das kann einige Minuten dauern) …")
    with torch.no_grad():
        sources = apply_model(model, mix[None], device="cpu", split=True, overlap=0.25, progress=False)[0]
    sources = sources * (ref.std() + 1e-8) + ref.mean()

    name, invert = spec
    idx = model.sources.index(name)
    if invert:
        out = sum(sources[i] for i in range(len(model.sources)) if i != idx)
    else:
        out = sources[idx]

    dst = out_dir / f"stem_{'no_' if invert else ''}{name}.wav"
    sf.write(str(dst), out.numpy().T, sr)
    return dst


# ---------------------------------------------------------------------------
# 3. Tempo & Tonart
# ---------------------------------------------------------------------------

def detect_tempo(wav: Path) -> tuple[float, float]:
    """Gibt (BPM, Zeitpunkt des ersten Schlags) zurück."""
    import librosa

    y, sr = librosa.load(str(wav), sr=22050, mono=True, duration=600)
    tempo, beats = librosa.beat.beat_track(y=y, sr=sr)
    bpm = float(np.atleast_1d(tempo)[0]) or 120.0
    first = float(librosa.frames_to_time(beats[0], sr=sr)) if len(beats) else 0.0
    # in einen sinnvollen Bereich falten
    while bpm < 60:
        bpm *= 2
    while bpm > 200:
        bpm /= 2
    return round(bpm, 1), first


def detect_key(notes: list[pretty_midi.Note]) -> tuple[int, bool, str]:
    """Krumhansl-Schmuckler. Gibt (Grundton, ist_moll, Name) zurück."""
    hist = np.zeros(12)
    for n in notes:
        hist[n.pitch % 12] += (n.end - n.start) * (n.velocity / 127)
    if hist.sum() == 0:
        return 0, False, "unbekannt"
    best = (-2.0, 0, False)
    for root in range(12):
        for minor, profile in ((False, _MAJOR_PROFILE), (True, _MINOR_PROFILE)):
            r = np.corrcoef(hist, np.roll(profile, root))[0, 1]
            if r > best[0]:
                best = (r, root, minor)
    _, root, minor = best
    return root, minor, f"{NOTE_NAMES[root]}-{'Moll' if minor else 'Dur'}"


def parse_key(label: str) -> Optional[tuple[int, bool]]:
    if not label or label == "Automatisch":
        return None
    name, mode = label.split("-")
    return NOTE_NAMES.index(name), mode == "Moll"


# ---------------------------------------------------------------------------
# 4. Transkription
# ---------------------------------------------------------------------------

_BP_MODEL = None


def _basic_pitch_model():
    global _BP_MODEL
    if _BP_MODEL is None:
        from basic_pitch import FilenameSuffix, build_icassp_2022_model_path
        from basic_pitch.inference import Model
        _BP_MODEL = Model(build_icassp_2022_model_path(FilenameSuffix.onnx))
    return _BP_MODEL


def transcribe(wav: Path, opts: Options, bpm: float) -> pretty_midi.PrettyMIDI:
    from basic_pitch.inference import predict

    _, midi, _ = predict(
        str(wav),
        _basic_pitch_model(),
        onset_threshold=opts.onset_threshold,
        frame_threshold=opts.frame_threshold,
        minimum_note_length=opts.min_note_ms,
        minimum_frequency=opts.min_freq or None,
        maximum_frequency=opts.max_freq or None,
        multiple_pitch_bends=opts.multiple_pitch_bends,
        melodia_trick=opts.melodia_trick,
        midi_tempo=bpm,
    )
    return midi


# ---------------------------------------------------------------------------
# 5. Nachbearbeitung
# ---------------------------------------------------------------------------

def _snap_pitch(pitch: int, root: int, minor: bool) -> int:
    scale = {(root + s) % 12 for s in (_MINOR_SCALE if minor else _MAJOR_SCALE)}
    if pitch % 12 in scale:
        return pitch
    for d in (-1, 1, -2, 2):  # bevorzugt nach unten
        if (pitch + d) % 12 in scale:
            return pitch + d
    return pitch


def _make_mono(notes: list[pretty_midi.Note]) -> list[pretty_midi.Note]:
    """Skyline: bei Überlappung gewinnt die höhere Note, frühere Noten werden gekürzt."""
    notes = sorted(notes, key=lambda n: (n.start, -n.pitch))
    out: list[pretty_midi.Note] = []
    for n in notes:
        if out and n.start < out[-1].end - 1e-4:
            prev = out[-1]
            if n.start - prev.start < 0.03:  # praktisch gleichzeitig: höhere Note gewinnt
                if n.pitch > prev.pitch:
                    out[-1] = n
                continue
            prev.end = n.start
        out.append(n)
    return [n for n in out if n.end - n.start > 0.01]


def _merge_repeats(notes: list[pretty_midi.Note], gap: float) -> list[pretty_midi.Note]:
    by_pitch: dict[int, list[pretty_midi.Note]] = {}
    for n in sorted(notes, key=lambda n: n.start):
        lst = by_pitch.setdefault(n.pitch, [])
        if lst and n.start - lst[-1].end <= gap:
            lst[-1].end = max(lst[-1].end, n.end)
            lst[-1].velocity = max(lst[-1].velocity, n.velocity)
        else:
            lst.append(n)
    return [n for lst in by_pitch.values() for n in lst]


def _quantize(notes: list[pretty_midi.Note], grid: float, anchor: float, strength: float,
              ends: bool) -> list[pretty_midi.Note]:
    def q(t: float) -> float:
        target = anchor + round((t - anchor) / grid) * grid
        return max(0.0, t + (target - t) * strength)

    for n in notes:
        dur = n.end - n.start
        n.start = q(n.start)
        n.end = q(n.end) if ends else n.start + dur
        if n.end - n.start < grid * 0.5 * strength or n.end <= n.start:
            n.end = n.start + max(grid * strength, dur * (1 - strength), 0.02)
    # doppelte Noten nach Quantisierung entfernen
    seen, out = set(), []
    for n in sorted(notes, key=lambda n: (n.start, n.pitch, -n.velocity)):
        k = (round(n.start, 4), n.pitch)
        if k not in seen:
            seen.add(k)
            out.append(n)
    return out


def postprocess(midi: pretty_midi.PrettyMIDI, opts: Options, bpm: float, beat_anchor: float,
                key: Optional[tuple[int, bool]]) -> pretty_midi.PrettyMIDI:
    out = pretty_midi.PrettyMIDI(initial_tempo=bpm)

    if opts.keep_pitch_bends and not opts.split_hands:
        groups = [(inst.notes, inst.pitch_bends) for inst in midi.instruments]
    else:
        groups = [([n for inst in midi.instruments for n in inst.notes], [])]

    grid = QUANTIZE_GRIDS.get(opts.quantize)
    for notes, bends in groups:
        notes = [pretty_midi.Note(n.velocity, n.pitch, n.start, n.end) for n in notes]

        notes = [n for n in notes if n.velocity >= opts.min_velocity]
        for n in notes:
            n.pitch = int(np.clip(n.pitch + opts.transpose, 0, 127))
        if opts.snap_to_key and key is not None:
            for n in notes:
                n.pitch = _snap_pitch(n.pitch, (key[0] + opts.transpose) % 12, key[1])
        notes = [n for n in notes if opts.note_low <= n.pitch <= opts.note_high]
        if opts.merge_gap_ms > 0:
            notes = _merge_repeats(notes, opts.merge_gap_ms / 1000)
        if opts.mono:
            notes = _make_mono(notes)
        if grid:
            notes = _quantize(notes, grid * 60.0 / bpm, beat_anchor, opts.quantize_strength / 100,
                              opts.quantize_ends)

        for n in notes:
            if opts.velocity_mode == "Fest":
                n.velocity = opts.fixed_velocity
            elif opts.velocity_mode == "Skaliert":
                n.velocity = int(np.clip(n.velocity * opts.velocity_scale / 100, 1, 127))

        notes.sort(key=lambda n: (n.start, n.pitch))
        if opts.split_hands:
            right = pretty_midi.Instrument(opts.program, name="Rechte Hand / Hoch")
            left = pretty_midi.Instrument(opts.program, name="Linke Hand / Tief")
            for n in notes:
                (right if n.pitch >= opts.split_point else left).notes.append(n)
            out.instruments += [i for i in (right, left) if i.notes]
        else:
            inst = pretty_midi.Instrument(opts.program, name=pretty_midi.program_to_instrument_name(opts.program))
            inst.notes = notes
            if opts.keep_pitch_bends:
                inst.pitch_bends = list(bends)
            out.instruments.append(inst)
    return out


# ---------------------------------------------------------------------------
# 6. Ausgaben
# ---------------------------------------------------------------------------

def synthesize(midi: pretty_midi.PrettyMIDI, sr: int = 22050) -> np.ndarray:
    """Einfacher, klavierähnlicher Synthesizer (ohne externe SoundFonts)."""
    length = midi.get_end_time() + 1.0
    buf = np.zeros(int(length * sr) + 1, dtype=np.float32)
    for inst in midi.instruments:
        for n in inst.notes:
            dur = max(n.end - n.start, 0.05)
            tail = 0.25
            t = np.arange(int((dur + tail) * sr)) / sr
            f = pretty_midi.note_number_to_hz(n.pitch)
            wave = (np.sin(2 * np.pi * f * t) + 0.4 * np.sin(4 * np.pi * f * t)
                    + 0.15 * np.sin(6 * np.pi * f * t))
            env = np.exp(-t * (1.5 + f / 600))
            env *= np.minimum(1.0, t / 0.005)
            release = np.clip(1 - (t - dur) / tail, 0, 1)
            env *= np.where(t > dur, release, 1.0)
            seg = (wave * env * (n.velocity / 127) * 0.2).astype(np.float32)
            i0 = int(n.start * sr)
            seg = seg[: len(buf) - i0]
            buf[i0 : i0 + len(seg)] += seg
    peak = np.abs(buf).max()
    if peak > 0.99:
        buf *= 0.99 / peak
    return buf


def save_pianoroll(midi: pretty_midi.PrettyMIDI, path: Path, title: str) -> Path:
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.collections import PatchCollection
    from matplotlib.patches import Rectangle

    notes = [n for inst in midi.instruments for n in inst.notes]
    fig, ax = plt.subplots(figsize=(14, 5), dpi=110)
    fig.patch.set_facecolor("#14161c")
    ax.set_facecolor("#1b1e26")
    if notes:
        lo = min(n.pitch for n in notes) - 2
        hi = max(n.pitch for n in notes) + 2
        for p in range(lo, hi + 1):
            if p % 12 in (1, 3, 6, 8, 10):
                ax.axhspan(p - 0.5, p + 0.5, color="#171a21", zorder=0)
        colors = ["#5eead4", "#f9a8d4", "#fcd34d", "#93c5fd"]
        for k, inst in enumerate(midi.instruments):
            rects = [Rectangle((n.start, n.pitch - 0.4), n.end - n.start, 0.8) for n in inst.notes]
            alphas = [0.35 + 0.65 * n.velocity / 127 for n in inst.notes]
            pc = PatchCollection(rects, facecolor=colors[k % len(colors)], edgecolor="none")
            pc.set_alpha(None)
            pc.set_facecolors([(*matplotlib.colors.to_rgb(colors[k % len(colors)]), a) for a in alphas])
            ax.add_collection(pc)
        ax.set_xlim(0, midi.get_end_time() + 0.2)
        ax.set_ylim(lo - 0.5, hi + 0.5)
        ticks = [p for p in range(lo, hi + 1) if p % 12 == 0]
        ax.set_yticks(ticks, [note_name(p) for p in ticks])
    ax.set_xlabel("Zeit (s)", color="#c9ccd4")
    ax.set_title(title, color="#e5e7eb", loc="left", fontsize=11)
    ax.tick_params(colors="#9aa0ab")
    for s in ax.spines.values():
        s.set_color("#2a2e38")
    ax.grid(axis="x", color="#262a34", linewidth=0.6)
    fig.tight_layout()
    fig.savefig(path, facecolor=fig.get_facecolor())
    plt.close(fig)
    return path


def save_csv(midi: pretty_midi.PrettyMIDI, path: Path) -> Path:
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, delimiter=";")
        w.writerow(["Spur", "Start (s)", "Ende (s)", "Dauer (s)", "MIDI-Note", "Name", "Velocity"])
        for inst in midi.instruments:
            for n in sorted(inst.notes, key=lambda n: n.start):
                w.writerow([inst.name, f"{n.start:.3f}", f"{n.end:.3f}", f"{n.end - n.start:.3f}",
                            n.pitch, note_name(n.pitch), n.velocity])
    return path


# ---------------------------------------------------------------------------
# Gesamtablauf
# ---------------------------------------------------------------------------

# Zwischenspeicher, damit beim erneuten Umwandeln mit anderen Optionen
# nicht neu heruntergeladen bzw. getrennt werden muss.
_DL_CACHE: dict[str, tuple[Path, str]] = {}
_STEM_CACHE: dict[tuple[str, str], Path] = {}

def convert(source: str | Path, work_dir: Path, opts: Options, *, is_url: bool = False,
            title: Optional[str] = None, cookies_file: Optional[str] = None,
            cookies_browser: Optional[str] = None, progress: ProgressFn = _noop) -> Result:
    work_dir.mkdir(parents=True, exist_ok=True)

    if is_url:
        progress(0.02, "Hole Audio vom Link …")
        cached = _DL_CACHE.get(str(source))
        if cached and cached[0].exists():
            src, dl_title = cached
        else:
            src, dl_title = download_audio(str(source), work_dir / "dl", cookies_file, cookies_browser, progress)
            _DL_CACHE[str(source)] = (src, dl_title)
        title = title or dl_title
    else:
        src = Path(source)
        title = title or src.stem
    name = safe_name(title)

    progress(0.2, "Bereite Audio vor …")
    wav = prepare_audio(src, work_dir, opts)
    if STEMS.get(opts.stem) is not None:
        # Stem-Trennung ist langsam: Ergebnis für gleiche Eingabe wiederverwenden
        cache_key = (hashlib.md5(wav.read_bytes()).hexdigest(), opts.stem)
        cached_stem = _STEM_CACHE.get(cache_key)
        if cached_stem and cached_stem.exists():
            wav = cached_stem
        else:
            wav = separate_stem(wav, work_dir, opts.stem, progress)
            _STEM_CACHE[cache_key] = wav

    progress(0.55, "Ermittle Tempo …")
    if opts.auto_tempo:
        bpm, anchor = detect_tempo(wav)
    else:
        bpm, anchor = float(opts.bpm), 0.0

    progress(0.6, "Erkenne Noten (Basic Pitch) …")
    raw = transcribe(wav, opts, bpm)

    raw_notes = [n for inst in raw.instruments for n in inst.notes]
    key = parse_key(opts.key_override)
    root, minor, key_label = detect_key(raw_notes)
    if key is None:
        key = (root, minor)
    else:
        key_label = opts.key_override + " (manuell)"

    progress(0.8, "Nachbearbeitung …")
    midi = postprocess(raw, opts, bpm, anchor, key)

    midi_path = work_dir / f"{name}.mid"
    midi.write(str(midi_path))

    audio_out = work_dir / f"{name}_audio.{opts.audio_format}"
    _run_ffmpeg(["-i", str(wav), *(["-b:a", "192k"] if opts.audio_format == "mp3" else []), str(audio_out)])

    result = Result(title=title, midi_path=midi_path, source_audio=audio_out)

    if opts.render_preview:
        progress(0.88, "Erzeuge Vorhör-Audio …")
        synth = synthesize(midi)
        prev = work_dir / f"{name}_midi_vorschau.wav"
        sf.write(str(prev), synth, 22050)
        result.preview_audio = prev

        import librosa
        orig, _ = librosa.load(str(wav), sr=22050, mono=True)
        n = max(len(orig), len(synth))
        mix = np.zeros((n, 2), dtype=np.float32)
        mix[: len(orig), 0] = orig * 0.8
        mix[: len(synth), 1] = synth
        mix[:, :] = mix[:, :] * 0.6 + mix[:, ::-1] * 0.4  # leicht gemischtes Stereo
        mix_path = work_dir / f"{name}_vergleich.wav"
        sf.write(str(mix_path), mix, 22050)
        result.mix_audio = mix_path

    progress(0.95, "Erstelle Pianoroll …")
    result.pianoroll = save_pianoroll(midi, work_dir / f"{name}_pianoroll.png", title)
    result.csv_path = save_csv(midi, work_dir / f"{name}_noten.csv")

    notes = [n for inst in midi.instruments for n in inst.notes]
    result.info = {
        "Titel": title,
        "Länge": f"{midi.get_end_time():.1f} s",
        "Tempo": f"{bpm:g} BPM" + (" (erkannt)" if opts.auto_tempo else ""),
        "Tonart": key_label,
        "Noten": len(notes),
        "Spuren": len(midi.instruments),
        "Tonumfang": (f"{note_name(min(n.pitch for n in notes))} – {note_name(max(n.pitch for n in notes))}"
                      if notes else "–"),
        "Quelle": opts.stem,
    }
    progress(1.0, "Fertig")
    return result
