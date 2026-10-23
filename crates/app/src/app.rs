//! Die Oberfläche.

use crate::pipeline::{Done, Input, Pipeline, Step};
use crate::platform::{self, Inbox};
use crate::player::Player;
use audio2midi_core::export::{self, TRACK_COLORS};
use audio2midi_core::model::SAMPLE_RATE;
use audio2midi_core::post::{note_name, presets, Key, Quantize, VelocityMode, GM_NAMES};
use audio2midi_core::{audio, Analysis, Audio, DetectParams, PostOptions, PrepOptions, RawNote, Song};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::sync::Arc;

const ACCENT: Color32 = Color32::from_rgb(94, 234, 212);
#[cfg(not(target_arch = "wasm32"))]
const BROWSERS: [&str; 9] = ["", "chrome", "firefox", "edge", "brave", "safari", "opera", "vivaldi", "chromium"];

#[derive(Clone, Copy, PartialEq)]
enum PlayKind {
    Midi,
    Original,
    Compare,
}

enum Job {
    #[cfg(not(target_arch = "wasm32"))]
    Thread { rx: std::sync::mpsc::Receiver<Msg>, progress: (f32, String) },
    #[cfg(target_arch = "wasm32")]
    Steps { pipeline: Pipeline, progress: (f32, String) },
}

#[cfg(not(target_arch = "wasm32"))]
enum Msg {
    Progress(f32, String),
    Done(Done),
    Error(String),
}

enum Source {
    #[cfg(not(target_arch = "wasm32"))]
    Link(String),
    Bytes(Vec<u8>, String),
    Decoded(Arc<Audio>),
}

struct Loaded {
    title: String,
    original: Arc<Audio>,
    prep: PrepOptions,
    analysis: Analysis,
}

struct Cached {
    detect: DetectParams,
    post: PostOptions,
    raw: Vec<RawNote>,
    song: Song,
    generation: u64,
}

pub struct App {
    inbox: Inbox,
    #[cfg(not(target_arch = "wasm32"))]
    link: String,
    #[cfg(not(target_arch = "wasm32"))]
    cookies_browser: usize,
    #[cfg(not(target_arch = "wasm32"))]
    cookies_file: Option<std::path::PathBuf>,
    prep: PrepOptions,
    detect: DetectParams,
    post: PostOptions,
    preset: usize,
    job: Option<Job>,
    loaded: Option<Loaded>,
    cache: Option<Cached>,
    generation: u64,
    /// (Generation, Art) → Stereo-Puffer
    audio_cache: Vec<(u64, PlayKind, Arc<Vec<f32>>)>,
    player: Player,
    playing: Option<PlayKind>,
    status: Option<String>,
    error: Option<String>,
    /// Pixel pro Sekunde
    zoom: f32,
    /// Linker Rand der Ansicht in Sekunden
    view_start: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = Color32::from_rgb(20, 120, 110);
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals);
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        // Datei aus der Kommandozeile („Öffnen mit …“ bzw. Datei auf das Programm ziehen)
        let inbox: Inbox = Default::default();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| p.is_file()) {
            if let Ok(bytes) = std::fs::read(&path) {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
                inbox.lock().unwrap().push(platform::PickedFile { name, bytes });
            }
        }
        App {
            inbox,
            #[cfg(not(target_arch = "wasm32"))]
            link: String::new(),
            #[cfg(not(target_arch = "wasm32"))]
            cookies_browser: 0,
            #[cfg(not(target_arch = "wasm32"))]
            cookies_file: None,
            prep: PrepOptions::default(),
            detect: DetectParams::default(),
            post: PostOptions::default(),
            preset: 0,
            job: None,
            loaded: None,
            cache: None,
            generation: 0,
            audio_cache: Vec::new(),
            player: Player::new(),
            playing: None,
            status: None,
            error: None,
            zoom: 0.0,
            view_start: 0.0,
        }
    }

    // ------------------------------------------------------------------
    // Verarbeitung

    fn start(&mut self, ctx: &egui::Context, title: String, source: Source) {
        self.error = None;
        self.status = None;
        self.player.stop();
        self.playing = None;
        let prep = self.prep.clone();

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let cookies = audio2midi_core::download::LinkOptions {
                cookies_file: self.cookies_file.clone(),
                cookies_browser: Some(BROWSERS[self.cookies_browser].to_string()).filter(|s| !s.is_empty()),
            };
            std::thread::spawn(move || {
                let send = |m: Msg| {
                    let _ = tx.send(m);
                    ctx.request_repaint();
                };
                let result = (|| -> anyhow::Result<Done> {
                    let (title, input) = match source {
                        Source::Link(url) => {
                            let d = audio2midi_core::download::download(&url, &cookies, |p, m| {
                                send(Msg::Progress(p * 0.3, m.to_string()));
                            })?;
                            (d.title, Input::Bytes { bytes: d.bytes, ext: d.extension })
                        }
                        Source::Bytes(b, ext) => (title, Input::Bytes { bytes: b, ext }),
                        Source::Decoded(a) => (title, Input::Decoded(a)),
                    };
                    let mut p = Pipeline::new(title, input, prep);
                    loop {
                        let (f, m) = p.status();
                        send(Msg::Progress(f, m.to_string()));
                        if let Step::Done(d) = p.step()? {
                            return Ok(d);
                        }
                    }
                })();
                send(match result {
                    Ok(d) => Msg::Done(d),
                    Err(e) => Msg::Error(format!("{e:#}")),
                });
            });
            self.job = Some(Job::Thread { rx, progress: (0.0, "Starte …".into()) });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let input = match source {
                Source::Bytes(b, ext) => Input::Bytes { bytes: b, ext },
                Source::Decoded(a) => Input::Decoded(a),
            };
            let pipeline = Pipeline::new(title, input, prep);
            let (f, m) = pipeline.status();
            self.job = Some(Job::Steps { pipeline, progress: (f, m.to_string()) });
            ctx.request_repaint();
        }
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else { return };
        let mut finished: Option<Result<Done, String>> = None;
        match job {
            #[cfg(not(target_arch = "wasm32"))]
            Job::Thread { rx, progress } => {
                for msg in rx.try_iter() {
                    match msg {
                        Msg::Progress(p, m) => *progress = (p, m),
                        Msg::Done(d) => finished = Some(Ok(d)),
                        Msg::Error(e) => finished = Some(Err(e)),
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            Job::Steps { pipeline, progress } => {
                // Ein Schritt pro Bild, damit die Seite bedienbar bleibt
                match pipeline.step() {
                    Ok(Step::Progress(p, m)) => *progress = (p, m.to_string()),
                    Ok(Step::Done(d)) => finished = Some(Ok(d)),
                    Err(e) => finished = Some(Err(format!("{e:#}"))),
                }
                ctx.request_repaint();
            }
        }
        let _ = ctx;
        match finished {
            Some(Ok(d)) => {
                self.job = None;
                self.cache = None;
                self.audio_cache.clear();
                self.view_start = 0.0;
                self.zoom = 0.0;
                self.loaded = Some(Loaded { title: d.title, original: d.original, prep: d.prep, analysis: d.analysis });
            }
            Some(Err(e)) => {
                self.job = None;
                self.error = Some(e);
            }
            None => {}
        }
    }

    fn take_inputs(&mut self, ctx: &egui::Context) {
        let picked: Vec<_> = self.inbox.lock().unwrap().drain(..).collect();
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let mut files: Vec<(String, Vec<u8>)> = picked.into_iter().map(|f| (f.name, f.bytes)).collect();
        for f in dropped {
            let name = f.path().file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
            #[cfg(not(target_arch = "wasm32"))]
            if let Ok(bytes) = f.bytes() {
                files.push((name, bytes));
            }
            // Im Browser lassen sich Dateien nur asynchron lesen
            #[cfg(target_arch = "wasm32")]
            {
                let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(bytes) = f.bytes_async().await {
                        inbox.lock().unwrap().push(platform::PickedFile { name, bytes });
                        ctx.request_repaint();
                    }
                });
            }
        }
        if let Some((name, bytes)) = files.pop() {
            if self.job.is_none() {
                let (stem, ext) = match name.rsplit_once('.') {
                    Some((s, e)) => (s.to_string(), e.to_lowercase()),
                    None => (name.clone(), String::new()),
                };
                self.start(ctx, stem, Source::Bytes(bytes, ext));
            }
        }
    }

    /// Noten neu berechnen, wenn sich Einstellungen geändert haben (ohne neuen Modell-Lauf).
    fn refresh_song(&mut self) {
        let Some(l) = &self.loaded else { return };
        let fresh = match &self.cache {
            Some(c) if c.detect == self.detect && c.post == self.post => return,
            Some(c) if c.detect == self.detect => {
                let raw = std::mem::take(&mut self.cache.as_mut().unwrap().raw);
                let song = l.analysis.song_from_raw(&raw, &self.post);
                (raw, song)
            }
            _ => {
                let raw = l.analysis.raw_notes(&self.detect);
                let song = l.analysis.song_from_raw(&raw, &self.post);
                (raw, song)
            }
        };
        self.generation += 1;
        self.audio_cache.retain(|(_, k, _)| *k == PlayKind::Original);
        self.cache = Some(Cached {
            detect: self.detect.clone(),
            post: self.post.clone(),
            raw: fresh.0,
            song: fresh.1,
            generation: self.generation,
        });
        // Läuft gerade MIDI oder Vergleich, mit neuem Stand weiterspielen
        if let (Some(kind), Some(pos)) = (self.playing, self.player.position()) {
            if kind != PlayKind::Original {
                self.play(kind, pos);
            }
        }
    }

    fn buffer(&mut self, kind: PlayKind) -> Option<Arc<Vec<f32>>> {
        let generation = if kind == PlayKind::Original { 0 } else { self.cache.as_ref()?.generation };
        if let Some((_, _, b)) = self.audio_cache.iter().find(|(g, k, _)| *g == generation && *k == kind) {
            return Some(b.clone());
        }
        let l = self.loaded.as_ref()?;
        let orig = &l.analysis.audio.samples;
        let stereo = |m: &[f32]| m.iter().flat_map(|s| [*s, *s]).collect::<Vec<f32>>();
        let buf = match kind {
            PlayKind::Original => stereo(orig),
            PlayKind::Midi => stereo(&export::synthesize(&self.cache.as_ref()?.song, SAMPLE_RATE)),
            PlayKind::Compare => export::comparison_mix(orig, &export::synthesize(&self.cache.as_ref()?.song, SAMPLE_RATE)),
        };
        let buf = Arc::new(buf);
        self.audio_cache.push((generation, kind, buf.clone()));
        Some(buf)
    }

    fn play(&mut self, kind: PlayKind, from: f64) {
        if let Some(buf) = self.buffer(kind) {
            self.player.play(buf, SAMPLE_RATE, from);
            self.playing = Some(kind);
        }
    }

    // ------------------------------------------------------------------
    // Optionen (linke Spalte)

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Einstellungen");
        ui.label(RichText::new("Änderungen wirken sofort – außer beim Zuschnitt.").small().weak());
        ui.add_space(4.0);

        let all = presets();
        egui::ComboBox::from_label("Voreinstellung")
            .selected_text(all[self.preset].name)
            .width(200.0)
            .show_ui(ui, |ui| {
                for (i, p) in all.iter().enumerate() {
                    if ui.selectable_label(self.preset == i, p.name).clicked() {
                        self.preset = i;
                        self.detect = p.detect.clone();
                        self.post.mono = p.mono;
                        self.post.program = p.program;
                    }
                }
            });
        ui.add_space(6.0);

        section(ui, "✂  Zuschnitt & Vorbereitung", false, |ui| {
            ui.horizontal(|ui| {
                ui.label("Start");
                ui.add(egui::DragValue::new(&mut self.prep.start).range(0.0..=36000.0).speed(0.1).suffix(" s"));
                ui.label("Ende");
                ui.add(egui::DragValue::new(&mut self.prep.end).range(0.0..=36000.0).speed(0.1).suffix(" s"));
            });
            ui.label(RichText::new("Ende 0 = bis zum Schluss").small().weak());
            ui.checkbox(&mut self.prep.normalize, "Lautstärke angleichen");
            let changed = self.loaded.as_ref().is_some_and(|l| l.prep != self.prep);
            if changed {
                let btn = ui.add_enabled(self.job.is_none(), egui::Button::new("↻ Neu analysieren"));
                if btn.clicked() {
                    let l = self.loaded.as_ref().unwrap();
                    let (title, a) = (l.title.clone(), l.original.clone());
                    self.start(ui.ctx(), title, Source::Decoded(a));
                }
            }
        });

        section(ui, "🎯  Notenerkennung", true, |ui| {
            let d = &mut self.detect;
            ui.add(egui::Slider::new(&mut d.onset_threshold, 0.05..=0.95).step_by(0.01).text("Einsatz-Schwelle"))
                .on_hover_text("Höher = weniger, dafür sicherere Notenanfänge");
            ui.add(egui::Slider::new(&mut d.frame_threshold, 0.05..=0.95).step_by(0.01).text("Noten-Schwelle"))
                .on_hover_text("Höher = weniger Noten und kürzere Haltedauer");
            ui.add(egui::Slider::new(&mut d.min_note_ms, 10.0..=500.0).step_by(1.0).suffix(" ms").text("Mindestlänge"));
            ui.horizontal(|ui| {
                ui.label("Frequenz");
                ui.add(egui::DragValue::new(&mut d.min_freq).range(0.0..=8000.0).speed(5.0).suffix(" Hz"));
                ui.label("bis");
                ui.add(egui::DragValue::new(&mut d.max_freq).range(0.0..=8000.0).speed(5.0).suffix(" Hz"));
            });
            ui.label(RichText::new("0 Hz = keine Grenze").small().weak());
            ui.checkbox(&mut d.melodia_trick, "Melodia-Trick (findet mehr gehaltene Töne)");
        });

        section(ui, "⏱  Tempo & Quantisierung", true, |ui| {
            let p = &mut self.post;
            ui.checkbox(&mut p.auto_tempo, "Tempo automatisch erkennen");
            ui.add_enabled(!p.auto_tempo, egui::Slider::new(&mut p.bpm, 30.0..=300.0).step_by(0.5).suffix(" BPM").text("Tempo"));
            egui::ComboBox::from_label("Raster")
                .selected_text(p.quantize.label())
                .show_ui(ui, |ui| {
                    for q in Quantize::ALL {
                        ui.selectable_value(&mut p.quantize, q, q.label());
                    }
                });
            let on = p.quantize != Quantize::Off;
            ui.add_enabled(on, egui::Slider::new(&mut p.quantize_strength, 0.0..=100.0).step_by(5.0).suffix(" %").text("Stärke"));
            ui.add_enabled(on, egui::Checkbox::new(&mut p.quantize_ends, "Notenenden mitquantisieren"));
        });

        section(ui, "🎼  Tonhöhe & Tonart", false, |ui| {
            let p = &mut self.post;
            ui.add(egui::Slider::new(&mut p.transpose, -24..=24).text("Transponieren").suffix(" HT"));
            let label = p.key.map(|k| k.name()).unwrap_or_else(|| "Automatisch".into());
            egui::ComboBox::from_label("Tonart").selected_text(label).height(300.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut p.key, None, "Automatisch");
                for k in Key::all() {
                    ui.selectable_value(&mut p.key, Some(k), k.name());
                }
            });
            ui.checkbox(&mut p.snap_to_key, "Falsche Töne auf die Tonleiter korrigieren");
            ui.checkbox(&mut p.mono, "Einstimmig (nur Melodie / höchster Ton)");
            ui.add(egui::Slider::new(&mut p.note_low, 0..=127).custom_formatter(|v, _| note_name(v as u8)).text("Tiefster Ton"));
            ui.add(egui::Slider::new(&mut p.note_high, 0..=127).custom_formatter(|v, _| note_name(v as u8)).text("Höchster Ton"));
        });

        section(ui, "🔊  Dynamik & Aufräumen", false, |ui| {
            let p = &mut self.post;
            ui.horizontal(|ui| {
                ui.label("Anschlag");
                ui.selectable_value(&mut p.velocity_mode, VelocityMode::Original, "Original");
                ui.selectable_value(&mut p.velocity_mode, VelocityMode::Fixed, "Fest");
                ui.selectable_value(&mut p.velocity_mode, VelocityMode::Scaled, "Skaliert");
            });
            match p.velocity_mode {
                VelocityMode::Fixed => {
                    ui.add(egui::Slider::new(&mut p.fixed_velocity, 1..=127).text("Velocity"));
                }
                VelocityMode::Scaled => {
                    ui.add(egui::Slider::new(&mut p.velocity_scale, 10.0..=200.0).step_by(5.0).suffix(" %").text("Faktor"));
                }
                VelocityMode::Original => {}
            }
            ui.add(egui::Slider::new(&mut p.min_velocity, 0..=127).text("Leise Noten entfernen"));
            ui.add(egui::Slider::new(&mut p.merge_gap_ms, 0.0..=200.0).step_by(5.0).suffix(" ms").text("Wiederholungen verbinden"));
        });

        section(ui, "💾  MIDI-Ausgabe", false, |ui| {
            let p = &mut self.post;
            egui::ComboBox::from_label("Instrument")
                .selected_text(format!("{:03} {}", p.program, GM_NAMES[p.program as usize]))
                .width(210.0)
                .height(320.0)
                .show_ui(ui, |ui| {
                    for (i, name) in GM_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut p.program, i as u8, format!("{i:03} {name}"));
                    }
                });
            ui.checkbox(&mut p.split_hands, "In zwei Spuren teilen (rechte/linke Hand)");
            ui.add_enabled(
                p.split_hands,
                egui::Slider::new(&mut p.split_point, 21..=108).custom_formatter(|v, _| note_name(v as u8)).text("Teilungspunkt"),
            );
            ui.checkbox(&mut p.keep_pitch_bends, "Pitch-Bends behalten")
                .on_hover_text("Nur ohne Quantisierung und ohne Spurteilung");
            self.detect.multiple_pitch_bends = p.keep_pitch_bends;
        });
        ui.add_space(12.0);
    }

    // ------------------------------------------------------------------
    // Hauptbereich

    fn source_ui(&mut self, ui: &mut egui::Ui) {
        let busy = self.job.is_some();
        egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let open = ui.add_enabled(!busy, egui::Button::new(RichText::new("📂  Datei öffnen").size(15.0)).min_size(Vec2::new(0.0, 30.0)));
                if open.clicked() {
                    platform::pick_audio(&self.inbox, ui.ctx());
                }
                ui.label(RichText::new("oder Datei hierher ziehen  ·  MP3, WAV, FLAC, OGG, M4A, MP4 …").weak());
            });
            ui.add_space(8.0);

            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 110.0).max(120.0);
                    let edit = ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.link)
                            .hint_text("Link einfügen: YouTube, SoundCloud, Bandcamp …")
                            .desired_width(w),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let go = ui.add_enabled(!busy && !self.link.trim().is_empty(), egui::Button::new("⬇  Laden"));
                    if go.clicked() || (enter && !self.link.trim().is_empty() && !busy) {
                        let url = self.link.trim().to_string();
                        self.start(ui.ctx(), String::new(), Source::Link(url));
                    }
                });
                egui::CollapsingHeader::new(RichText::new("Link-Optionen (bei YouTube-Anmeldefehler)").small()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("Cookies aus Browser")
                            .selected_text(if self.cookies_browser == 0 { "keine" } else { BROWSERS[self.cookies_browser] })
                            .show_ui(ui, |ui| {
                                for (i, b) in BROWSERS.iter().enumerate() {
                                    ui.selectable_value(&mut self.cookies_browser, i, if i == 0 { "keine" } else { b });
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        if ui.button("cookies.txt wählen …").clicked() {
                            self.cookies_file = platform::pick_cookies();
                        }
                        if let Some(f) = &self.cookies_file {
                            ui.label(f.file_name().and_then(|n| n.to_str()).unwrap_or(""));
                            if ui.small_button("✖").clicked() {
                                self.cookies_file = None;
                            }
                        }
                    });
                });
            }
            #[cfg(target_arch = "wasm32")]
            {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("🔗 Links (YouTube, SoundCloud …) gehen nur in der Desktop-App –").weak());
                    ui.hyperlink_to("hier herunterladen", "https://github.com/Mailo037/audio2midi/releases");
                });
                ui.label(RichText::new("Alles läuft lokal in deinem Browser, nichts wird hochgeladen.").small().weak());
            }

            if let Some(job) = &self.job {
                let (p, m) = match job {
                    #[cfg(not(target_arch = "wasm32"))]
                    Job::Thread { progress, .. } => progress,
                    #[cfg(target_arch = "wasm32")]
                    Job::Steps { progress, .. } => progress,
                };
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(*p).text(format!("{m}  {:.0} %", p * 100.0)).animate(true));
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(255, 120, 120), format!("⚠ {e}"));
            }
            if let Some(e) = &self.player.error {
                ui.colored_label(Color32::from_rgb(255, 120, 120), format!("⚠ {e}"));
            }
        });
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(l), Some(c)) = (&self.loaded, &self.cache) else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("🎹").size(48.0));
                ui.label(RichText::new("Öffne eine Audiodatei, um loszulegen.").size(16.0).weak());
            });
            return;
        };
        let song = &c.song;
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&l.title).size(18.0).strong());
            ui.add_space(10.0);
            let range = song.pitch_range().map(|(a, b)| format!("{} – {}", note_name(a), note_name(b))).unwrap_or("–".into());
            for (k, v) in [
                ("Länge", format!("{:.1} s", l.analysis.audio.duration())),
                ("Tempo", format!("{} BPM{}", song.bpm, if song.tempo_detected { " (erkannt)" } else { "" })),
                ("Tonart", format!("{}{}", song.key.name(), if song.key_detected { " (erkannt)" } else { "" })),
                ("Noten", song.note_count().to_string()),
                ("Spuren", song.tracks.len().to_string()),
                ("Umfang", range),
            ] {
                ui.label(RichText::new(k).weak());
                ui.label(RichText::new(v).strong());
                ui.add_space(6.0);
            }
        });
        ui.add_space(6.0);

        let pos = self.player.position();
        if pos.is_none() {
            self.playing = None;
        }
        let mut play: Option<PlayKind> = None;
        let mut stop = false;
        let mut save: Option<(String, Vec<u8>, &'static str)> = None;
        let name = safe_name(&l.title);
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [(PlayKind::Midi, "▶  MIDI"), (PlayKind::Original, "▶  Original"), (PlayKind::Compare, "▶  Vergleich")] {
                let active = self.playing == Some(kind);
                let btn = egui::Button::new(if active { RichText::new(label).color(ACCENT) } else { RichText::new(label) }).selected(active);
                let r = ui.add(btn);
                let r = if kind == PlayKind::Compare { r.on_hover_text("Original und MIDI gleichzeitig") } else { r };
                if r.clicked() {
                    play = Some(kind);
                }
            }
            if ui.add_enabled(self.playing.is_some(), egui::Button::new("⏹  Stopp")).clicked() {
                stop = true;
            }
            ui.separator();
            if ui.add(egui::Button::new(RichText::new("💾  MIDI speichern").strong()).fill(Color32::from_rgb(15, 95, 88))).clicked() {
                save = Some((format!("{name}.mid"), export::midi_bytes(song), "audio/midi"));
            }
            ui.menu_button("Weitere Exporte ⏷", |ui| {
                if ui.button("Notenliste (CSV)").clicked() {
                    save = Some((format!("{name}_noten.csv"), export::csv(song).into_bytes(), "text/csv"));
                }
                if ui.button("Pianoroll (SVG)").clicked() {
                    save = Some((format!("{name}_pianoroll.svg"), export::pianoroll_svg(song, &l.title).into_bytes(), "image/svg+xml"));
                }
                if ui.button("MIDI-Vorschau (WAV)").clicked() {
                    let s = export::synthesize(song, SAMPLE_RATE);
                    save = Some((format!("{name}_midi.wav"), audio::wav_bytes(&s, SAMPLE_RATE, 1), "audio/wav"));
                }
                if ui.button("Verwendete Audiospur (WAV)").clicked() {
                    save = Some((format!("{name}_audio.wav"), audio::wav_bytes(&l.analysis.audio.samples, SAMPLE_RATE, 1), "audio/wav"));
                }
            });
        });
        ui.add_space(6.0);

        let anchor = if self.post.auto_tempo { l.analysis.tempo.1 } else { 0.0 };
        let duration = l.analysis.audio.duration();
        let mut seek: Option<f64> = None;
        roll_ui(ui, song, duration, anchor, pos, &mut self.zoom, &mut self.view_start, &mut seek);

        if let Some(kind) = play {
            self.play(kind, 0.0);
        }
        if let Some(t) = seek {
            let kind = self.playing.unwrap_or(PlayKind::Midi);
            self.play(kind, t);
        }
        if stop {
            self.player.stop();
            self.playing = None;
        }
        if let Some((n, b, mime)) = save {
            if let Some(msg) = platform::save(&n, b, mime) {
                self.status = Some(msg);
            }
        }
        if self.playing.is_some() {
            ui.ctx().request_repaint();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.take_inputs(&ctx);
        self.poll_job(&ctx);

        egui::Panel::top("kopf").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎹 Audio → MIDI").size(20.0).strong().color(ACCENT));
                ui.label(RichText::new("Noten aus Musik erkennen und als MIDI speichern").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to("GitHub", "https://github.com/Mailo037/audio2midi");
                    if let Some(s) = &self.status {
                        ui.label(RichText::new(s).small().weak());
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::Panel::left("optionen").resizable(true).default_size(330.0).min_size(260.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.options_ui(ui));
        });
        self.refresh_song();

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.source_ui(ui);
            self.results_ui(ui);
        });

        // Hinweis beim Hineinziehen
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            let rect = ctx.content_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
            painter.text(rect.center(), Align2::CENTER_CENTER, "Loslassen zum Umwandeln", FontId::proportional(28.0), Color32::WHITE);
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(open).show(ui, |ui| {
        ui.spacing_mut().slider_width = 150.0;
        add(ui);
        ui.add_space(4.0);
    });
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
    let n = n.trim().to_string();
    if n.is_empty() { "audio".into() } else { n.chars().take(100).collect() }
}

/// Pianoroll mit Taktraster, Zoom (Strg + Mausrad), Verschieben (ziehen/Mausrad) und Abspielmarke.
#[allow(clippy::too_many_arguments)]
fn roll_ui(
    ui: &mut egui::Ui,
    song: &Song,
    duration: f64,
    anchor: f64,
    playhead: Option<f64>,
    zoom: &mut f32,
    view_start: &mut f64,
    seek: &mut Option<f64>,
) {
    let key_w = 38.0;
    let total = duration.max(song.end_time()).max(1.0);
    // Einpassen, bevor der Zoom-Regler den Wert begrenzt
    if *zoom <= 0.0 {
        *zoom = ((ui.available_width() - key_w - 4.0).max(100.0) as f64 / total) as f32;
        *view_start = 0.0;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Pianoroll").strong());
        ui.label(RichText::new("Klicken = ab hier abspielen · Ziehen/Mausrad = verschieben · Strg+Mausrad = Zoom").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Einpassen").clicked() {
                *zoom = 0.0;
            }
            ui.add(egui::Slider::new(zoom, 1.0..=600.0).logarithmic(true).show_value(false).text("Zoom"));
        });
    });
    let size = Vec2::new(ui.available_width(), ui.available_height().max(220.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(20, 22, 28));

    let area = Rect::from_min_max(Pos2::new(rect.left() + key_w, rect.top() + 4.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 20.0));

    // Eingaben: Zoom um den Mauszeiger, Verschieben
    if resp.hovered() {
        let (scroll, ctrl) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers.ctrl || i.modifiers.command));
        if ctrl && scroll.y != 0.0 {
            let mx = resp.hover_pos().map(|p| p.x).unwrap_or(area.center().x).clamp(area.left(), area.right());
            let t_at = *view_start + ((mx - area.left()) / *zoom) as f64;
            *zoom = (*zoom * (1.0 + scroll.y * 0.004)).clamp(1.0, 600.0);
            *view_start = t_at - ((mx - area.left()) / *zoom) as f64;
        } else {
            let d = if scroll.x != 0.0 { scroll.x } else { scroll.y };
            *view_start -= (d / *zoom) as f64;
        }
    }
    if resp.dragged() {
        *view_start -= (resp.drag_delta().x / *zoom) as f64;
    }
    let visible = (area.width() / *zoom) as f64;
    // Abspielmarke im Blick behalten
    if let Some(t) = playhead {
        if !resp.dragged() && (t > *view_start + visible * 0.95 || t < *view_start) {
            *view_start = t - visible * 0.1;
        }
    }
    *view_start = view_start.clamp(0.0, (total - visible * 0.5).max(0.0));
    let x_of = |t: f64| area.left() + ((t - *view_start) as f32) * *zoom;
    let t_of = |x: f32| *view_start + ((x - area.left()) / *zoom) as f64;

    let (lo, hi) = song.pitch_range().map(|(a, b)| (a.saturating_sub(3), (b + 3).min(127))).unwrap_or((48, 72));
    let rows = (hi - lo + 1) as f32;
    let rh = area.height() / rows;
    let y_of = |p: u8| area.top() + (hi - p) as f32 * rh;

    // Hintergrund: schwarze Tasten, Oktavlinien
    for p in lo..=hi {
        let y = y_of(p);
        let row = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(area.width(), rh));
        if matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
            painter.rect_filled(row, 0.0, Color32::from_rgb(26, 29, 37));
        }
        if p % 12 == 0 {
            painter.line_segment([Pos2::new(area.left(), y + rh), Pos2::new(area.right(), y + rh)], Stroke::new(1.0, Color32::from_rgb(40, 44, 54)));
            if rh * 12.0 > 30.0 || p % 24 == 0 {
                painter.text(Pos2::new(rect.left() + 6.0, y + rh * 0.5), Align2::LEFT_CENTER, note_name(p), FontId::proportional(11.0), Color32::from_gray(150));
            }
        }
    }

    // Taktraster aus dem Tempo
    let beat = 60.0 / song.bpm;
    let beat_px = beat as f32 * *zoom;
    let step = if beat_px > 12.0 { 1 } else if beat_px * 4.0 > 12.0 { 4 } else { 16 };
    let first = ((*view_start - anchor) / beat).floor() as i64;
    let last = ((*view_start + visible - anchor) / beat).ceil() as i64;
    for b in first..=last {
        if b.rem_euclid(step) != 0 {
            continue;
        }
        let t = anchor + b as f64 * beat;
        let x = x_of(t);
        if x < area.left() || x > area.right() {
            continue;
        }
        let bar = b.rem_euclid(4) == 0;
        let col = if bar { Color32::from_rgb(52, 57, 70) } else { Color32::from_rgb(34, 38, 47) };
        painter.line_segment([Pos2::new(x, area.top()), Pos2::new(x, area.bottom())], Stroke::new(1.0, col));
        if bar && (beat_px * 4.0 > 34.0 || b.rem_euclid(16) == 0) {
            painter.text(Pos2::new(x + 3.0, area.top() + 7.0), Align2::LEFT_CENTER, format!("{}", b.div_euclid(4) + 1), FontId::proportional(10.0), Color32::from_gray(110));
        }
    }
    // Zeitachse in Sekunden
    let sec_step = [1.0, 2.0, 5.0, 10.0, 30.0, 60.0].into_iter().find(|s| (*s as f32) * *zoom > 60.0).unwrap_or(120.0);
    let mut t = (*view_start / sec_step).floor() * sec_step;
    while t < *view_start + visible {
        let x = x_of(t);
        if x >= area.left() {
            painter.text(Pos2::new(x, rect.bottom() - 10.0), Align2::CENTER_CENTER, format!("{t:.0}s"), FontId::proportional(10.0), Color32::from_gray(90));
        }
        t += sec_step;
    }

    // Noten
    let hover = resp.hover_pos();
    let mut hovered: Option<String> = None;
    for (i, tr) in song.tracks.iter().enumerate() {
        let [r, g, b] = TRACK_COLORS[i % TRACK_COLORS.len()];
        for n in &tr.notes {
            if n.end < *view_start || n.start > *view_start + visible {
                continue;
            }
            let x0 = x_of(n.start).max(area.left());
            let x1 = x_of(n.end).min(area.right()).max(x0 + 1.5);
            let y = y_of(n.pitch);
            let nr = Rect::from_min_max(Pos2::new(x0, y + rh * 0.08), Pos2::new(x1, y + rh * 0.92));
            let alpha = (90.0 + 165.0 * n.velocity as f32 / 127.0) as u8;
            painter.rect_filled(nr, (rh * 0.2).min(3.0), Color32::from_rgba_unmultiplied(r, g, b, alpha));
            if hover.is_some_and(|h| nr.expand(1.0).contains(h)) {
                painter.rect_stroke(nr, 2.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Outside);
                hovered = Some(format!("{}  ·  {:.2}–{:.2} s  ·  Velocity {}  ·  {}", note_name(n.pitch), n.start, n.end, n.velocity, tr.name));
            }
        }
    }
    if song.note_count() == 0 {
        painter.text(area.center(), Align2::CENTER_CENTER, "Keine Noten erkannt – Schwellen niedriger einstellen", FontId::proportional(15.0), Color32::from_gray(140));
    }

    if let Some(t) = playhead {
        let x = x_of(t);
        if x >= area.left() && x <= area.right() {
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(2.0, ACCENT));
        }
    }
    if let Some(h) = hover {
        if h.x >= area.left() {
            painter.line_segment([Pos2::new(h.x, area.top()), Pos2::new(h.x, area.bottom())], Stroke::new(1.0, Color32::from_white_alpha(25)));
        }
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if p.x >= area.left() {
                *seek = Some(t_of(p.x).max(0.0));
            }
        }
    }
    if let Some(text) = hovered {
        resp.on_hover_text_at_pointer(text);
    }
}
