# 🎹 Audio → MIDI

Link einfügen **oder** Datei hochladen → das Programm holt die Audiospur, erkennt die Noten
(Spotify **Basic Pitch**, läuft lokal) und gibt eine **MIDI-Datei** aus.

## Starten

Voraussetzungen: **Python 3.10/3.11** und **ffmpeg**.

| System  | ffmpeg installieren          | Programm starten |
|---------|------------------------------|------------------|
| Windows | `winget install ffmpeg`      | `start.bat` doppelklicken |
| macOS   | `brew install ffmpeg`        | `./start.sh` |
| Linux   | `sudo apt install ffmpeg`    | `./start.sh` |

Beim ersten Start wird alles automatisch installiert, danach öffnet sich
`http://127.0.0.1:7860` im Browser. Ergebnisse landen im Ordner `ausgabe/`.

## Was es kann

**Quellen**
- Links von YouTube, SoundCloud, Bandcamp, Vimeo, direkten MP3-Links u. v. m. (über yt-dlp) – auch mehrere auf einmal
- Datei-Upload: MP3, WAV, FLAC, M4A, OGG, OPUS **und Videos** (MP4, MOV, MKV …) – auch mehrere (gibt zusätzlich ein ZIP)
- YouTube-Anmeldefehler („Sign in to confirm you're not a bot“): Cookies aus dem Browser übernehmen oder `cookies.txt` hochladen

**Optionen**
| Bereich | Einstellungen |
|---|---|
| Voreinstellungen | Standard, Klavier, Gesang/Melodie, Bass, Gitarre, Pfeifen/Flöte |
| Spurtrennung (optional, Demucs) | nur Gesang, Instrumental, nur Bass, andere Instrumente, ohne Schlagzeug |
| Zuschnitt | Start/Ende in Sekunden, Lautstärke normalisieren |
| Notenerkennung | Einsatz-Empfindlichkeit, Noten-Schwelle, Mindestnotenlänge, Frequenzbereich, Melodia-Trick, Pitch-Bends |
| Tempo | automatische BPM-Erkennung oder fest |
| Quantisierung | 1/4 … 1/32 inkl. Triolen, Stärke in %, Notenenden ja/nein |
| Tonhöhe | Transponieren ±24, Tonart automatisch/manuell, falsche Töne auf Tonleiter einrasten, Notenbereich begrenzen, einstimmig (Melodie) |
| Dynamik | Velocity original/fest/skaliert, leise Noten entfernen, wiederholte Töne verbinden |
| MIDI | General-MIDI-Instrument (alle 128), in zwei Spuren teilen (rechte/linke Hand), Pitch-Bends behalten |

**Ausgaben**: `.mid`, Pianoroll-Bild, Noten als CSV, MIDI zum Anhören, Vergleich Original + MIDI,
die extrahierte Audiospur (MP3/WAV/FLAC), Info zu Tempo, Tonart, Tonumfang.

## Kommandozeile

```bash
.venv/bin/python cli.py lied.mp3
.venv/bin/python cli.py "https://soundcloud.com/…" --preset Klavier --quantize 1/16 --split 60
.venv/bin/python cli.py "https://youtu.be/…" --stem Gesang --mono --cookies-from-browser firefox
.venv/bin/python cli.py --help
```
(Windows: `.venv\Scripts\python cli.py …`)

## Tipps für gute Ergebnisse
- Basic Pitch funktioniert am besten mit **einem Instrument** oder einer klaren Melodie.
  Bei kompletten Songs vorher die passende Spur abtrennen (Gesang/Bass/Andere).
- Zu viele Geisternoten → *Noten-Schwelle* und *Einsatz-Empfindlichkeit* erhöhen.
- Für Notensatz-Programme (MuseScore …) Quantisierung auf 1/16 und ggf. „In zwei Spuren teilen“.
- Schlagzeug kann nicht in Noten umgewandelt werden.

Bitte nur Material herunterladen, für das du die Rechte hast bzw. das die Nutzungsbedingungen der Plattform erlauben.
