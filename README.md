# 🎹 Audio → MIDI

Turn music into notes: open a file or paste a link, the program detects the notes and saves them as a **MIDI file**.
Written entirely in **Rust**. Note detection is an exact port of Spotify's [Basic Pitch](https://github.com/spotify/basic-pitch)
and produces the same notes as the Python original, followed by a clean-up pass that removes ghost notes.

## Three versions, one codebase

| Version | What it can do | Installation |
|---|---|---|
| **In the browser** – [mailo037.github.io/audio2midi](https://mailo037.github.io/audio2midi/) | Convert files. Everything runs locally in the browser, nothing is uploaded. | none |
| **Desktop app** (Windows/macOS/Linux) | additionally **links** (YouTube, SoundCloud, Bandcamp …) | download one file and start it |
| **Command line** `audio2midi` | everything, including many files at once | one file |

Ready-made programs are available under [Releases](https://github.com/Mailo037/audio2midi/releases).
For links, the app automatically downloads [yt-dlp](https://github.com/yt-dlp/yt-dlp) the first time.
If [ffmpeg](https://ffmpeg.org) is installed, even more sites and formats work.

## Features

**Sources:** MP3, WAV, FLAC, OGG, M4A/AAC, MP4/MOV, MKV, AIFF – via dialog, drag & drop, “Open with …” or link.

**Options** (apply instantly, without analyzing again):

| Area | Settings |
|---|---|
| Presets | Default, Piano, Vocals/melody, Bass, Guitar, Whistle/flute |
| Trim | start/end, normalize volume |
| Note detection | onset threshold, note threshold, minimum length, frequency range, Melodia trick, remove ghost notes |
| Tempo | automatic BPM detection or fixed |
| Quantization | 1/4 … 1/32 including triplets, strength in %, note ends yes/no |
| Pitch | transpose ±24, key automatic/manual, snap wrong notes to the scale, limit the range, monophonic |
| Dynamics | velocity original/fixed/scaled, remove quiet notes, merge repeated notes |
| MIDI | all 128 General MIDI instruments, two tracks (right/left hand), pitch bends |

**Result:** piano roll with bar grid and zoom, listen to the MIDI, the original, or both at once. Export as MIDI, CSV, SVG and WAV.

### Remove ghost notes

The raw model output often contains notes that weren't played: overtones an octave (or octave + fifth, two octaves …)
above a real note, sub-octaves below it, and short blips far above or below the music. The clean-up pass
(“Remove ghost notes”, default 50 %) removes

- notes in an overtone interval that sound at the same time as a clearly louder note,
- notes far outside the range of what is played in the surrounding ±3 seconds, unless they are long and loud,
- very short notes that are much quieter than their neighbours, or that stand completely on their own.

0 % gives the unmodified Basic Pitch result.

## Command line

```bash
audio2midi song.mp3
audio2midi "https://soundcloud.com/…" --preset piano --quantize 1/16 --split 60
audio2midi "https://youtu.be/…" --mono --cookies-from-browser firefox
audio2midi *.wav --transpose -2 --key "A minor" --snap-to-key -o my_midis
audio2midi song.mp3 --cleanup 80
audio2midi --help
```

For each file you get the `.mid`, a note list (`.csv`), a piano roll (`.svg`), the analyzed audio and preview WAVs.

## Building it yourself

```bash
cargo run --release -p audio2midi-app        # desktop app
cargo run --release -p audio2midi-cli -- …    # command line
./build-web.sh                                # browser version into site/
```

Linux needs `libasound2-dev`. For the web version: `rustup target add wasm32-unknown-unknown` and
`cargo install wasm-bindgen-cli` (same version as in `Cargo.lock`).

| Folder | Contents |
|---|---|
| `crates/core` | audio decoding, model (tract/ONNX), note detection, clean-up, post-processing, MIDI/CSV/SVG/WAV |
| `crates/app` | user interface (egui), runs natively and as WebAssembly |
| `crates/cli` | command line |
| `web/` | HTML shell of the web version |
| `.github/workflows` | builds the web version (GitHub Pages) and the programs for Windows, Linux and macOS on every push |

## Tips

- It works best with **a single instrument** or a clear melody. Full songs with drums are less accurate.
- Still too many stray notes → raise *Remove ghost notes*, or the *note threshold* and *onset threshold*.
- Real notes missing → lower *Remove ghost notes* or the thresholds.
- For notation software (MuseScore …): quantization 1/16 and “Split into two tracks”.

Please only use material you have the rights to, or that the platform's terms of use allow.

## License

Apache-2.0. The detection model comes from [spotify/basic-pitch](https://github.com/spotify/basic-pitch) (Apache-2.0).
