//! Dateien öffnen und speichern – nativ über Systemdialoge, im Browser über Upload/Download.

use std::sync::{Arc, Mutex};

pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Ablage für asynchron ausgewählte Dateien (der Browser liefert sie erst später).
pub type Inbox = Arc<Mutex<Vec<PickedFile>>>;

pub const AUDIO_EXTENSIONS: &[&str] =
    &["mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "mp4", "m4v", "mov", "mkv", "webm", "aif", "aiff", "caf"];

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_audio(inbox: &Inbox, ctx: &egui::Context) {
    let inbox = inbox.clone();
    let ctx = ctx.clone();
    // Eigener Thread, damit das Fenster während des Dialogs weiter gezeichnet wird
    std::thread::spawn(move || {
        if let Some(path) = rfd::FileDialog::new().add_filter("Audio / Video", AUDIO_EXTENSIONS).pick_file() {
            if let Ok(bytes) = std::fs::read(&path) {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
                inbox.lock().unwrap().push(PickedFile { name, bytes });
                ctx.request_repaint();
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub fn pick_audio(inbox: &Inbox, ctx: &egui::Context) {
    let inbox = inbox.clone();
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        if let Some(file) = rfd::AsyncFileDialog::new().add_filter("Audio / Video", AUDIO_EXTENSIONS).pick_file().await {
            let bytes = file.read().await;
            inbox.lock().unwrap().push(PickedFile { name: file.file_name(), bytes });
            ctx.request_repaint();
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_cookies() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().add_filter("cookies.txt", &["txt"]).pick_file()
}

/// Datei speichern. Gibt eine Meldung für die Statuszeile zurück.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(name: &str, bytes: Vec<u8>, _mime: &str) -> Option<String> {
    let ext = name.rsplit('.').next().unwrap_or("");
    let path = rfd::FileDialog::new().set_file_name(name).add_filter(ext, &[ext]).save_file()?;
    Some(match std::fs::write(&path, bytes) {
        Ok(()) => format!("Gespeichert: {}", path.display()),
        Err(e) => format!("Speichern fehlgeschlagen: {e}"),
    })
}

#[cfg(target_arch = "wasm32")]
pub fn save(name: &str, bytes: Vec<u8>, mime: &str) -> Option<String> {
    use wasm_bindgen::JsCast;
    let run = || -> Result<(), wasm_bindgen::JsValue> {
        let arr = js_sys::Uint8Array::from(bytes.as_slice());
        let parts = js_sys::Array::of1(&arr);
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let doc = web_sys::window().unwrap().document().unwrap();
        let a: web_sys::HtmlAnchorElement = doc.create_element("a")?.dyn_into()?;
        a.set_href(&url);
        a.set_download(name);
        a.click();
        web_sys::Url::revoke_object_url(&url)?;
        Ok(())
    };
    Some(match run() {
        Ok(()) => format!("Heruntergeladen: {name}"),
        Err(e) => format!("Download fehlgeschlagen: {e:?}"),
    })
}
