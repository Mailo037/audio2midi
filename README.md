# 🎹 Audio → MIDI

Musik in Noten umwandeln: Datei öffnen oder Link einfügen, das Programm erkennt die Noten und speichert sie als **MIDI-Datei**.
Komplett in **Rust**. Die Notenerkennung ist eine exakte Portierung von Spotifys [Basic Pitch](https://github.com/spotify/basic-pitch)
und liefert dieselben Noten wie das Python-Original.

## Drei Varianten, ein Code

| Variante | Was geht | Installation |
|---|---|---|
| **Im Browser** – [mailo037.github.io/audio2midi](https://mailo037.github.io/audio2midi/) | Dateien umwandeln. Alles läuft lokal im Browser, nichts wird hochgeladen. | keine |
| **Desktop-App** (Windows/macOS/Linux) | zusätzlich **Links** (YouTube, SoundCloud, Bandcamp …) | eine Datei herunterladen und starten |
| **Kommandozeile** `audio2midi` | alles, auch viele Dateien auf einmal | eine Datei |

Fertige Programme liegen unter [Releases](https://github.com/Mailo037/audio2midi/releases).
Für Links lädt die App beim ersten Mal automatisch [yt-dlp](https://github.com/yt-dlp/yt-dlp) herunter.
Ist [ffmpeg](https://ffmpeg.org) installiert, funktionieren noch mehr Seiten und Formate.

## Funktionen

**Quellen:** MP3, WAV, FLAC, OGG, M4A/AAC, MP4/MOV, MKV, AIFF – per Dialog, Drag & Drop, „Öffnen mit …“ oder Link.

**Optionen** (wirken sofort, ohne neu zu analysieren):

| Bereich | Einstellungen |
|---|---|
| Voreinstellungen | Standard, Klavier, Gesang/Melodie, Bass, Gitarre, Pfeifen/Flöte |
| Zuschnitt | Start/Ende, Lautstärke angleichen |
| Notenerkennung | Einsatz-Schwelle, Noten-Schwelle, Mindestlänge, Frequenzbereich, Melodia-Trick |
| Tempo | automatische BPM-Erkennung oder fest |
| Quantisierung | 1/4 … 1/32 inkl. Triolen, Stärke in %, Notenenden ja/nein |
| Tonhöhe | Transponieren ±24, Tonart automatisch/manuell, falsche Töne auf Tonleiter korrigieren, Tonumfang begrenzen, einstimmig |
| Dynamik | Velocity original/fest/skaliert, leise Noten entfernen, Wiederholungen verbinden |
| MIDI | alle 128 General-MIDI-Instrumente, zwei Spuren (rechte/linke Hand), Pitch-Bends |

**Ergebnis:** Pianoroll mit Taktraster und Zoom, Anhören von MIDI, Original und beidem gleichzeitig. Export als MIDI, CSV, SVG und WAV.

## Kommandozeile

```bash
audio2midi lied.mp3
audio2midi "https://soundcloud.com/…" --preset Klavier --quantize 1/16 --split 60
audio2midi "https://youtu.be/…" --mono --cookies-from-browser firefox
audio2midi *.wav --transpose -2 --key A-Moll --snap-to-key -o meine_midis
audio2midi --help
```

Pro Datei entstehen `.mid`, Notenliste (`.csv`), Pianoroll (`.svg`), Audiospur und Vorhör-WAVs.

## Selbst bauen

```bash
cargo run --release -p audio2midi-app        # Desktop-App
cargo run --release -p audio2midi-cli -- …    # Kommandozeile
./build-web.sh                                # Browser-Version nach site/
```

Linux braucht `libasound2-dev`. Für die Web-Version: `rustup target add wasm32-unknown-unknown` und
`cargo install wasm-bindgen-cli` (gleiche Version wie in `Cargo.lock`).

| Ordner | Inhalt |
|---|---|
| `crates/core` | Audio dekodieren, Modell (tract/ONNX), Notenerkennung, Nachbearbeitung, MIDI/CSV/SVG/WAV |
| `crates/app` | Oberfläche (egui), läuft nativ und als WebAssembly |
| `crates/cli` | Kommandozeile |
| `web/` | HTML-Gerüst der Web-Version |
| `.github/workflows` | baut bei jedem Push die Web-Version (GitHub Pages) und die Programme für Windows, Linux und macOS |

## Tipps

- Am besten klappt es mit **einem Instrument** oder einer klaren Melodie. Komplette Songs mit Schlagzeug werden ungenau.
- Zu viele Geisternoten → *Noten-Schwelle* und *Einsatz-Schwelle* erhöhen.
- Für Notensatzprogramme (MuseScore …): Quantisierung 1/16 und „In zwei Spuren teilen“.

Bitte nur Material verwenden, für das du die Rechte hast bzw. das die Nutzungsbedingungen der Plattform erlauben.

## Lizenz

Apache-2.0. Das Erkennungsmodell stammt aus [spotify/basic-pitch](https://github.com/spotify/basic-pitch) (Apache-2.0).
