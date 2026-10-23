//! Verarbeitung in kleinen Schritten: nativ in einem eigenen Thread, im Browser
//! ein Schritt pro Bild (dort gibt es keine Threads).

use anyhow::Result;
use audio2midi_core::{audio, model::Transcription, prepare, Analysis, Audio, Model, PrepOptions};
use std::sync::{Arc, OnceLock};

static MODEL: OnceLock<Arc<Model>> = OnceLock::new();

fn model() -> Result<Arc<Model>> {
    if let Some(m) = MODEL.get() {
        return Ok(m.clone());
    }
    let m = Arc::new(Model::load()?);
    Ok(MODEL.get_or_init(|| m).clone())
}

pub enum Input {
    Bytes { bytes: Vec<u8>, ext: String },
    Decoded(Arc<Audio>),
}

enum Stage {
    Decode(Vec<u8>, String),
    Prepare,
    LoadModel,
    Run(Transcription),
    Finished,
}

pub struct Pipeline {
    pub title: String,
    stage: Stage,
    original: Option<Arc<Audio>>,
    prepared: Option<Audio>,
    prep: PrepOptions,
    model: Option<Arc<Model>>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum Step {
    Progress(f32, &'static str),
    Done(Done),
}

pub struct Done {
    pub title: String,
    pub original: Arc<Audio>,
    pub prep: PrepOptions,
    pub analysis: Analysis,
}

impl Pipeline {
    pub fn new(title: String, input: Input, prep: PrepOptions) -> Self {
        let (stage, original) = match input {
            Input::Bytes { bytes, ext } => (Stage::Decode(bytes, ext), None),
            Input::Decoded(a) => (Stage::Prepare, Some(a)),
        };
        Pipeline { title, stage, original, prepared: None, prep, model: None }
    }

    /// Text, der *während* des nächsten Schritts angezeigt wird.
    pub fn status(&self) -> (f32, &'static str) {
        match &self.stage {
            Stage::Decode(..) => (0.02, "Lese Audiodatei …"),
            Stage::Prepare => (0.06, "Bereite Audio vor …"),
            Stage::LoadModel => (0.1, "Lade Erkennungsmodell …"),
            Stage::Run(t) => (0.12 + 0.88 * t.progress(), "Erkenne Noten …"),
            Stage::Finished => (1.0, "Fertig"),
        }
    }

    pub fn step(&mut self) -> Result<Step> {
        match std::mem::replace(&mut self.stage, Stage::Finished) {
            Stage::Decode(bytes, ext) => {
                let a = audio::decode(bytes, Some(&ext))?;
                self.original = Some(Arc::new(a));
                self.stage = Stage::Prepare;
            }
            Stage::Prepare => {
                let a = prepare(self.original.as_ref().unwrap(), &self.prep);
                if a.samples.len() < 2205 {
                    anyhow::bail!("Der gewählte Ausschnitt ist zu kurz.");
                }
                self.prepared = Some(a);
                self.stage = Stage::LoadModel;
            }
            Stage::LoadModel => {
                self.model = Some(model()?);
                self.stage = Stage::Run(Transcription::new(&self.prepared.as_ref().unwrap().samples));
            }
            Stage::Run(mut t) => {
                if t.step(self.model.as_ref().unwrap())? {
                    let analysis = Analysis::new(self.prepared.take().unwrap(), t.finish());
                    return Ok(Step::Done(Done {
                        title: std::mem::take(&mut self.title),
                        original: self.original.take().unwrap(),
                        prep: self.prep.clone(),
                        analysis,
                    }));
                }
                self.stage = Stage::Run(t);
            }
            Stage::Finished => anyhow::bail!("Bereits fertig"),
        }
        let (p, m) = self.status();
        Ok(Step::Progress(p, m))
    }
}
