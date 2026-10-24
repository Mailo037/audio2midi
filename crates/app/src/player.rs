//! Audiowiedergabe: nativ über cpal, im Browser über die Web-Audio-API.
//! Abgespielt wird immer verschachteltes Stereo.

#[cfg(not(target_arch = "wasm32"))]
pub use native::Player;
#[cfg(target_arch = "wasm32")]
pub use web::Player;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        samples: Arc<Vec<f32>>,
        rate: f64,
        /// Position in Quell-Frames
        pos: f64,
        playing: bool,
    }

    pub struct Player {
        state: Arc<Mutex<State>>,
        stream: Option<cpal::Stream>,
        device_rate: f64,
        pub error: Option<String>,
    }

    impl Player {
        pub fn new() -> Self {
            Player { state: Default::default(), stream: None, device_rate: 44100.0, error: None }
        }

        fn ensure_stream(&mut self) -> anyhow::Result<()> {
            if self.stream.is_some() {
                return Ok(());
            }
            let host = cpal::default_host();
            let device = host.default_output_device().ok_or_else(|| anyhow::anyhow!("No audio output found"))?;
            let supported = device.default_output_config()?;
            let format = supported.sample_format();
            let config: cpal::StreamConfig = supported.into();
            self.device_rate = config.sample_rate as f64;
            let channels = config.channels as usize;
            let state = self.state.clone();
            let device_rate = self.device_rate;
            let err = |e| eprintln!("Audio error: {e}");
            macro_rules! build {
                ($t:ty, $conv:expr) => {
                    device.build_output_stream(
                        config.clone(),
                        move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                            let mut st = state.lock().unwrap();
                            for frame in data.chunks_mut(channels) {
                                let (l, r) = next_frame(&mut st, device_rate);
                                for (c, s) in frame.iter_mut().enumerate() {
                                    let v = if c % 2 == 0 { l } else { r };
                                    *s = $conv(v);
                                }
                            }
                        },
                        err,
                        None,
                    )?
                };
            }
            let stream = match format {
                cpal::SampleFormat::F32 => build!(f32, |v: f32| v),
                cpal::SampleFormat::I16 => build!(i16, |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16),
                cpal::SampleFormat::U16 => build!(u16, |v: f32| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16),
                other => anyhow::bail!("Unsupported audio format: {other:?}"),
            };
            stream.play()?;
            self.stream = Some(stream);
            Ok(())
        }

        pub fn play(&mut self, stereo: Arc<Vec<f32>>, rate: u32, from: f64) {
            if let Err(e) = self.ensure_stream() {
                self.error = Some(format!("Playback not possible: {e}"));
                return;
            }
            let mut st = self.state.lock().unwrap();
            st.samples = stereo;
            st.rate = rate as f64;
            st.pos = from.max(0.0) * rate as f64;
            st.playing = true;
        }

        pub fn stop(&mut self) {
            self.state.lock().unwrap().playing = false;
        }

        /// Aktuelle Position in Sekunden, falls gerade abgespielt wird.
        pub fn position(&self) -> Option<f64> {
            let st = self.state.lock().unwrap();
            st.playing.then(|| st.pos / st.rate)
        }
    }

    fn next_frame(st: &mut State, device_rate: f64) -> (f32, f32) {
        if !st.playing {
            return (0.0, 0.0);
        }
        let frames = st.samples.len() / 2;
        let i = st.pos as usize;
        if i + 1 >= frames {
            st.playing = false;
            return (0.0, 0.0);
        }
        let f = (st.pos - i as f64) as f32;
        let s = &st.samples;
        let l = s[2 * i] * (1.0 - f) + s[2 * i + 2] * f;
        let r = s[2 * i + 1] * (1.0 - f) + s[2 * i + 3] * f;
        st.pos += st.rate / device_rate;
        (l, r)
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::Arc;
    use web_sys::{AudioBufferSourceNode, AudioContext};

    pub struct Player {
        ctx: Option<AudioContext>,
        source: Option<AudioBufferSourceNode>,
        started_at: f64,
        offset: f64,
        duration: f64,
        pub error: Option<String>,
    }

    impl Player {
        pub fn new() -> Self {
            Player { ctx: None, source: None, started_at: 0.0, offset: 0.0, duration: 0.0, error: None }
        }

        pub fn play(&mut self, stereo: Arc<Vec<f32>>, rate: u32, from: f64) {
            self.stop();
            if let Err(e) = self.try_play(&stereo, rate, from) {
                self.error = Some(format!("Playback not possible: {e:?}"));
            }
        }

        fn try_play(&mut self, stereo: &[f32], rate: u32, from: f64) -> Result<(), wasm_bindgen::JsValue> {
            if self.ctx.is_none() {
                self.ctx = Some(AudioContext::new()?);
            }
            let ctx = self.ctx.as_ref().unwrap();
            let _ = ctx.resume();
            let frames = (stereo.len() / 2).max(1);
            let buf = ctx.create_buffer(2, frames as u32, rate as f32)?;
            for ch in 0..2 {
                let data: Vec<f32> = stereo.iter().skip(ch).step_by(2).copied().collect();
                buf.copy_to_channel(&data, ch as i32)?;
            }
            let src = ctx.create_buffer_source()?;
            src.set_buffer(Some(&buf));
            src.connect_with_audio_node(&ctx.destination())?;
            src.start_with_when_and_grain_offset(0.0, from.max(0.0))?;
            self.started_at = ctx.current_time();
            self.offset = from.max(0.0);
            self.duration = frames as f64 / rate as f64;
            self.source = Some(src);
            Ok(())
        }

        pub fn stop(&mut self) {
            if let Some(src) = self.source.take() {
                #[allow(deprecated)]
                let _ = src.stop();
            }
        }

        pub fn position(&self) -> Option<f64> {
            let ctx = self.ctx.as_ref()?;
            self.source.as_ref()?;
            let pos = self.offset + ctx.current_time() - self.started_at;
            (pos < self.duration).then_some(pos)
        }
    }
}
