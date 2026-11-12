//! Pocket TTS ONNX runtime.
//!
//! The exported graphs follow the reference Python cycle:
//!
//! 1. text token ids -> `text_conditioner` -> 1024-d embeddings;
//! 2. `flow_lm_main` (stateful KV cache) predicts conditioning + EOS logit;
//! 3. `flow_lm_flow` performs one LSD Euler step from Gaussian noise;
//! 4. `mimi_decoder` turns each 32-d latent frame into 1920 samples at 24 kHz.
//!
//! Russian Shtorm packages additionally encode the reference WAV with
//! `mimi_encoder` and prepend `bos_before_voice`. Full English packages load a
//! ready voice state (Alba) directly into the KV cache.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use half::f16;
use ort::session::Session;
use ort::value::{DynValue, Tensor, TensorElementType, ValueType};
use tracing::{debug, info};

use super::audio::prepare_reference_wav;
use super::descriptor::PocketModelDescriptor;
use super::safetensors::SafeTensorsFile;
use super::tokenizer::{
    split_text_chunks, PocketTokenizer, FRAMES_AFTER_EOS, MAX_TOKENS_PER_CHUNK,
    MIN_FRAMES_BEFORE_EOS,
};
use super::{PocketError, PocketVoiceSelection};

const CPU_THREADS: usize = 4;
/// Target output level after the soft gain compensation (ROADMAP-137).
///
/// A reference normalized to `RMS ~0.048` makes the model quieter, so the
/// synthesized PCM is lifted towards this level to stay comparable with the
/// other engines.
const TARGET_OUTPUT_RMS: f32 = 0.12;
/// Peak ceiling of the output limiter.
const OUTPUT_PEAK_LIMIT: f32 = 0.95;
/// Never boost more than this, so near-silent garbage is not amplified.
const MAX_OUTPUT_GAIN: f32 = 4.0;
/// Ignore tiny corrections: they are inaudible and only add distortion risk.
const OUTPUT_GAIN_DEADBAND: f32 = 0.05;

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct SynthStats {
    pub chunks: usize,
    pub frames: usize,
    pub eos_frame: Option<usize>,
    pub duration_seconds: f32,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct SynthOutput {
    pub wav: Vec<u8>,
    pub stats: SynthStats,
}

pub struct PocketRuntime {
    descriptor: PocketModelDescriptor,
    tokenizer: PocketTokenizer,
    main: Session,
    flow: Session,
    decoder: Session,
    encoder: Session,
    conditioner: Session,
    main_state_order: Vec<String>,
    main_state_specs: HashMap<String, StateSpec>,
    decoder_state_order: Vec<String>,
    decoder_state_specs: HashMap<String, StateSpec>,
    bos_before_voice: Option<Vec<f32>>,
    state_capacity: usize,
}

#[derive(Debug, Clone)]
struct StateSpec {
    dtype: TensorElementType,
    shape: Vec<i64>,
}

impl PocketRuntime {
    pub fn load(
        descriptor: &PocketModelDescriptor,
        cancel: &AtomicBool,
    ) -> Result<Self, PocketError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let main = load_session(&descriptor.graphs.main)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let flow = load_session(&descriptor.graphs.flow)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let decoder = load_session(&descriptor.graphs.decoder)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let encoder = load_session(&descriptor.graphs.encoder)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let conditioner = load_session(&descriptor.graphs.conditioner)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }

        validate_main_graph(&main)?;
        validate_flow_graph(&flow)?;
        validate_decoder_graph(&decoder)?;
        validate_encoder_graph(&encoder)?;
        validate_conditioner_graph(&conditioner)?;

        let (main_state_order, main_state_specs, state_capacity) = collect_states(&main, "state_")?;
        if main_state_order.len() != descriptor.num_layers * 3 {
            return Err(PocketError::Runtime(format!(
                "main graph has {} state inputs, expected {} for {} layers",
                main_state_order.len(),
                descriptor.num_layers * 3,
                descriptor.num_layers
            )));
        }
        if state_capacity == 0 {
            return Err(PocketError::Runtime(
                "main graph KV cache has zero capacity".to_string(),
            ));
        }
        let (decoder_state_order, decoder_state_specs, _) = collect_states(&decoder, "state_")?;

        let tokenizer = match descriptor.tokenizer_kind {
            super::PocketTokenizerKind::Json => {
                PocketTokenizer::json(&descriptor.tokenizer_path, descriptor.n_bins)?
            }
            super::PocketTokenizerKind::SentencePiece => {
                PocketTokenizer::sentencepiece(&descriptor.tokenizer_path, descriptor.n_bins)?
            }
        };

        let bos_before_voice = if descriptor.insert_bos_before_voice {
            Some(load_bos(&descriptor.weights_path)?)
        } else {
            None
        };

        info!(
            package = %descriptor.package_id,
            quantized = descriptor.graphs.quantized,
            layers = descriptor.num_layers,
            states = main_state_order.len(),
            capacity = state_capacity,
            vocab = tokenizer.vocab_size(),
            "Pocket TTS runtime loaded"
        );

        Ok(Self {
            descriptor: descriptor.clone(),
            tokenizer,
            main,
            flow,
            decoder,
            encoder,
            conditioner,
            main_state_order,
            main_state_specs,
            decoder_state_order,
            decoder_state_specs,
            bos_before_voice,
            state_capacity,
        })
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        voice: Option<&PocketVoiceSelection>,
        cancel: &AtomicBool,
    ) -> Result<SynthOutput, PocketError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let voice = match voice {
            Some(voice) => voice,
            None => {
                return Err(if self.descriptor.voices.is_empty() {
                    PocketError::ReferenceRequired
                } else {
                    PocketError::VoiceMissing("default".to_string())
                })
            }
        };
        let chunks = split_text_chunks(
            &self.tokenizer,
            &self.descriptor.language,
            text,
            MAX_TOKENS_PER_CHUNK,
        )?;
        let mut all_samples = Vec::new();
        let mut stats = SynthStats {
            chunks: chunks.len(),
            ..SynthStats::default()
        };

        for chunk in chunks {
            if cancel.load(Ordering::Relaxed) {
                return Err(PocketError::Cancelled);
            }
            let (mut samples, eos_frame) = self.synthesize_chunk(&chunk, voice, cancel)?;
            apply_fade_in(&mut samples, self.sample_rate());
            if let Some(frame) = eos_frame {
                stats.frames += frame as usize + FRAMES_AFTER_EOS as usize;
                if stats.eos_frame.is_none() {
                    stats.eos_frame = Some(frame as usize);
                }
            }
            all_samples.extend_from_slice(&samples);
        }

        if all_samples.is_empty() {
            return Err(PocketError::Generation(
                "generation produced no audio samples".to_string(),
            ));
        }
        if all_samples.iter().any(|sample| !sample.is_finite()) {
            return Err(PocketError::Generation(
                "generation produced non-finite samples".to_string(),
            ));
        }
        let rms = (all_samples
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / all_samples.len() as f64)
            .sqrt();
        if !rms.is_finite() || rms < 0.0001 {
            return Err(PocketError::Generation(
                "generation produced silence".to_string(),
            ));
        }
        compensate_output_gain(&mut all_samples);
        stats.duration_seconds = all_samples.len() as f32 / self.sample_rate() as f32;
        let wav = crate::audio::effects::encode_wav(&all_samples, self.sample_rate(), 1)
            .map_err(PocketError::Generation)?;
        Ok(SynthOutput { wav, stats })
    }

    fn sample_rate(&self) -> u32 {
        self.descriptor.sample_rate
    }

    fn frame_samples(&self) -> usize {
        ((self.descriptor.sample_rate as f32 / self.descriptor.frame_rate).round() as usize).max(1)
    }

    fn synthesize_chunk(
        &mut self,
        text: &str,
        voice: &PocketVoiceSelection,
        cancel: &AtomicBool,
    ) -> Result<(Vec<f32>, Option<u32>), PocketError> {
        let mut main_states = initial_states(&self.main_state_order, &self.main_state_specs)?;

        let start_offset = match voice {
            PocketVoiceSelection::VoiceState(voice_id) => {
                let descriptor_voice = self
                    .descriptor
                    .voice(voice_id)
                    .ok_or_else(|| PocketError::VoiceMissing(voice_id.clone()))?;
                self.apply_saved_voice_state(&descriptor_voice.state_path, &mut main_states)?
            }
            PocketVoiceSelection::ReferenceWav(path) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(PocketError::Cancelled);
                }
                let samples =
                    prepare_reference_wav(path, self.sample_rate(), self.frame_samples())?;
                if samples.is_empty() {
                    return Err(PocketError::ReferenceInvalid(
                        "reference is empty after preparation".to_string(),
                    ));
                }
                let voice_conditioning = self.encode_reference(&samples)?;
                let frames = voice_conditioning.len() / self.condition_dim()?;
                self.prefill_voice(&voice_conditioning, &mut main_states, cancel)?;
                frames
                    + if self.bos_before_voice.is_some() {
                        1
                    } else {
                        0
                    }
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let token_ids = self.tokenizer.encode(text)?;
        if token_ids.is_empty() {
            return Err(PocketError::Generation(
                "tokenizer produced no tokens".to_string(),
            ));
        }
        let text_embeddings = self.condition_text(&token_ids)?;
        let max_frames = self.max_generation_frames(token_ids.len());
        let required = start_offset + token_ids.len() + max_frames;
        if required > self.state_capacity {
            return Err(PocketError::Generation(format!(
                "generation needs {required} KV slots but the model has {}",
                self.state_capacity
            )));
        }
        self.prefill_text(&text_embeddings, &mut main_states, cancel)?;

        let mut decoder_states =
            initial_states(&self.decoder_state_order, &self.decoder_state_specs)?;
        let mut rng = NoiseRng::new();
        let mut sequence = nan_sequence(self.descriptor.latent_dim)?;
        let condition_dim = self.condition_dim()?;
        let mut eos_step: Option<u32> = None;
        let mut samples = Vec::new();
        let mut generated = 0u32;

        for step in 0..max_frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(PocketError::Cancelled);
            }
            let outputs = self.run_main(
                sequence,
                empty_text_embeddings(condition_dim)?,
                &mut main_states,
            )?;
            let conditioning = extract_named_f32(&outputs, "conditioning")?;
            let eos_logit = extract_named_f32(&outputs, "eos_logit")?;
            let eos = eos_logit
                .first()
                .copied()
                .map(|value| value > self.descriptor.eos_threshold)
                .unwrap_or(false);
            if eos && eos_step.is_none() && step as u32 >= MIN_FRAMES_BEFORE_EOS {
                eos_step = Some(step as u32);
            }
            if let Some(eos_step) = eos_step {
                if step as u32 >= eos_step + FRAMES_AFTER_EOS {
                    break;
                }
            }

            let noise = rng.normal(
                self.descriptor.temperature.sqrt(),
                self.descriptor.latent_dim,
            );
            let direction = self.run_flow(&conditioning, &noise)?;
            let next: Vec<f32> = noise
                .iter()
                .zip(direction.iter())
                .map(|(noise, direction)| noise + direction)
                .collect();
            if next.len() != self.descriptor.latent_dim {
                return Err(PocketError::Generation(format!(
                    "flow produced {} values, expected {}",
                    next.len(),
                    self.descriptor.latent_dim
                )));
            }
            sequence = Tensor::from_array((vec![1usize, 1, next.len()], next.clone()))
                .map_err(|error| PocketError::Generation(error.to_string()))?
                .into_dyn();
            let latent = Tensor::from_array((vec![1usize, 1, next.len()], next))
                .map_err(|error| PocketError::Generation(error.to_string()))?
                .into_dyn();
            let audio_outputs = run_states(
                &mut self.decoder,
                vec![("latent".to_string(), latent)],
                &mut decoder_states,
                &self.decoder_state_order,
            )?;
            let audio = extract_named_f32(&audio_outputs, "audio_frame")?;
            samples.extend_from_slice(&audio);
            generated += 1;
        }

        if eos_step.is_none() {
            return Err(PocketError::Generation(
                "generation reached maximum length without EOS".to_string(),
            ));
        }
        debug!(
            frames = generated,
            eos_frame = eos_step,
            samples = samples.len(),
            "Pocket TTS chunk generated"
        );
        Ok((samples, eos_step))
    }

    fn max_generation_frames(&self, token_count: usize) -> usize {
        let seconds = token_count as f32 / 3.0 + 2.0;
        let frames = (seconds * self.descriptor.frame_rate).ceil() as usize;
        frames.clamp(8, 1000)
    }

    fn encode_reference(&mut self, samples: &[f32]) -> Result<Vec<f32>, PocketError> {
        let audio = Tensor::from_array((vec![1usize, 1, samples.len()], samples.to_vec()))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let mut states = HashMap::new();
        let outputs = run_states(
            &mut self.encoder,
            vec![("audio".to_string(), audio)],
            &mut states,
            &[],
        )?;
        let (shape, values) = extract_f32(&outputs, "conditioning")?;
        if shape.len() != 3 || shape[0] != 1 {
            return Err(PocketError::Generation(format!(
                "mimi encoder returned shape {shape:?}, expected [1, frames, dim]"
            )));
        }
        Ok(values)
    }

    fn prefill_voice(
        &mut self,
        voice_conditioning: &[f32],
        states: &mut HashMap<String, DynValue>,
        cancel: &AtomicBool,
    ) -> Result<(), PocketError> {
        let dim = self.condition_dim()?;
        if !voice_conditioning.len().is_multiple_of(dim) {
            return Err(PocketError::Generation(
                "voice conditioning has an invalid length".to_string(),
            ));
        }
        let frames = voice_conditioning.len() / dim;
        let voice_frames = if self.bos_before_voice.is_some() {
            frames + 1
        } else {
            frames
        };
        let mut embeddings = Vec::with_capacity(voice_frames * dim);
        if let Some(bos) = &self.bos_before_voice {
            if bos.len() != dim {
                return Err(PocketError::Runtime(format!(
                    "bos_before_voice has {} values, expected {dim}",
                    bos.len()
                )));
            }
            embeddings.extend_from_slice(bos);
        }
        embeddings.extend_from_slice(voice_conditioning);
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let text = Tensor::from_array((vec![1usize, voice_frames, dim], embeddings))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let sequence = Tensor::from_array((
            vec![1usize, 0usize, self.descriptor.latent_dim],
            Vec::<f32>::new(),
        ))
        .map_err(|error| PocketError::Runtime(error.to_string()))?
        .into_dyn();
        let _ = self.run_main(sequence, text, states)?;
        Ok(())
    }

    fn prefill_text(
        &mut self,
        embeddings: &[f32],
        states: &mut HashMap<String, DynValue>,
        cancel: &AtomicBool,
    ) -> Result<(), PocketError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(PocketError::Cancelled);
        }
        let dim = self.condition_dim()?;
        if !embeddings.len().is_multiple_of(dim) {
            return Err(PocketError::Generation(
                "text embeddings have an invalid length".to_string(),
            ));
        }
        let tokens = embeddings.len() / dim;
        let text = Tensor::from_array((vec![1usize, tokens, dim], embeddings.to_vec()))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let sequence = Tensor::from_array((
            vec![1usize, 0usize, self.descriptor.latent_dim],
            Vec::<f32>::new(),
        ))
        .map_err(|error| PocketError::Runtime(error.to_string()))?
        .into_dyn();
        let _ = self.run_main(sequence, text, states)?;
        Ok(())
    }

    fn condition_text(&mut self, tokens: &[i64]) -> Result<Vec<f32>, PocketError> {
        let ids = Tensor::from_array((vec![1usize, tokens.len()], tokens.to_vec()))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let mut states = HashMap::new();
        let outputs = run_states(
            &mut self.conditioner,
            vec![("token_ids".to_string(), ids)],
            &mut states,
            &[],
        )?;
        let (shape, values) = extract_f32(&outputs, "embeddings")?;
        if shape.len() != 3 || shape[0] != 1 {
            return Err(PocketError::Generation(format!(
                "text conditioner returned shape {shape:?}, expected [1, tokens, dim]"
            )));
        }
        Ok(values)
    }

    fn condition_dim(&self) -> Result<usize, PocketError> {
        let output = self
            .conditioner
            .outputs()
            .iter()
            .find(|output| output.name() == "embeddings")
            .ok_or_else(|| {
                PocketError::Runtime("text conditioner has no embeddings".to_string())
            })?;
        match output.dtype() {
            ValueType::Tensor { shape, .. } if shape.len() == 3 => Ok(shape[2].max(0) as usize),
            _ => Err(PocketError::Runtime(
                "text conditioner embedding shape is not [B, T, D]".to_string(),
            )),
        }
    }

    fn run_main(
        &mut self,
        sequence: DynValue,
        text_embeddings: DynValue,
        states: &mut HashMap<String, DynValue>,
    ) -> Result<HashMap<String, DynValue>, PocketError> {
        run_states(
            &mut self.main,
            vec![
                ("sequence".to_string(), sequence),
                ("text_embeddings".to_string(), text_embeddings),
            ],
            states,
            &self.main_state_order,
        )
    }

    fn run_flow(&mut self, conditioning: &[f32], noise: &[f32]) -> Result<Vec<f32>, PocketError> {
        let c = Tensor::from_array((vec![conditioning.len()], conditioning.to_vec()))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let s = Tensor::from_array((vec![1usize, 1usize], vec![0.0f32]))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let t = Tensor::from_array((vec![1usize, 1usize], vec![1.0f32]))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let x = Tensor::from_array((vec![1usize, noise.len()], noise.to_vec()))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn();
        let mut states = HashMap::new();
        let outputs = run_states(
            &mut self.flow,
            vec![
                ("c".to_string(), c),
                ("s".to_string(), s),
                ("t".to_string(), t),
                ("x".to_string(), x),
            ],
            &mut states,
            &[],
        )?;
        let (shape, values) = extract_f32(&outputs, "flow_dir")?;
        if shape.iter().product::<usize>() != noise.len() {
            return Err(PocketError::Generation(format!(
                "flow_dir shape {shape:?} does not match latent size {}",
                noise.len()
            )));
        }
        Ok(values)
    }

    fn apply_saved_voice_state(
        &self,
        path: &Path,
        states: &mut HashMap<String, DynValue>,
    ) -> Result<usize, PocketError> {
        let mut file = SafeTensorsFile::open(path).map_err(PocketError::Package)?;
        file.validate_pocket_voice(self.descriptor.num_layers)
            .map_err(PocketError::Package)?;
        let mut start_offset = 0usize;
        for layer in 0..self.descriptor.num_layers {
            let module = format!("transformer.layers.{layer}.self_attn");
            let offset = file
                .read_i64_scalar(&format!("{module}/offset"))
                .map_err(PocketError::Package)?;
            if offset < 0 {
                return Err(PocketError::Package(
                    "voice state has a negative offset".to_string(),
                ));
            }
            let offset = offset as usize;
            start_offset = offset;
            let (cache_shape, cache_values) = file
                .read_f32(&format!("{module}/cache"))
                .map_err(PocketError::Package)?;
            if cache_shape.len() != 5 || cache_shape[0] != 2 || cache_shape[1] != 1 {
                return Err(PocketError::Package(format!(
                    "voice cache shape {cache_shape:?} is not [2, 1, frames, heads, dim]"
                )));
            }
            let heads = cache_shape[3];
            let dim = cache_shape[4];
            let cache_frames = cache_shape[2];
            if offset > cache_frames {
                return Err(PocketError::Package(
                    "voice state offset exceeds its cache length".to_string(),
                ));
            }
            let kind_stride = cache_frames.saturating_mul(heads).saturating_mul(dim);
            let state_k = format!("state_{}", layer * 3);
            let state_v = format!("state_{}", layer * 3 + 1);
            let state_offset = format!("state_{}", layer * 3 + 2);
            let spec_k = self
                .main_state_specs
                .get(&state_k)
                .ok_or_else(|| PocketError::Runtime(format!("missing {state_k}")))?;
            let spec_v = self
                .main_state_specs
                .get(&state_v)
                .ok_or_else(|| PocketError::Runtime(format!("missing {state_v}")))?;
            let (capacity, expected_heads, expected_dim) = match spec_k.shape.as_slice() {
                [1, capacity, model_heads, model_dim] => (
                    *capacity as usize,
                    *model_heads as usize,
                    *model_dim as usize,
                ),
                other => {
                    return Err(PocketError::Runtime(format!(
                        "{state_k} shape {other:?} is not [1, capacity, heads, dim]"
                    )))
                }
            };
            if heads != expected_heads || dim != expected_dim {
                return Err(PocketError::Package(
                    "voice cache dimensions do not match the main graph".to_string(),
                ));
            }
            if capacity < offset {
                return Err(PocketError::Package(
                    "voice state offset exceeds the main graph KV capacity".to_string(),
                ));
            }
            for (state_name, spec, kind) in [(&state_k, spec_k, 0usize), (&state_v, spec_v, 1usize)]
            {
                let mut values = vec![0.0f32; capacity * heads * dim];
                let source_base = kind * kind_stride;
                for frame in 0..offset {
                    let source = source_base + frame * heads * dim;
                    let target = frame * heads * dim;
                    let source_end = source + heads * dim;
                    if source_end > cache_values.len() {
                        return Err(PocketError::Package(
                            "voice cache tensor is truncated".to_string(),
                        ));
                    }
                    values[target..target + heads * dim]
                        .copy_from_slice(&cache_values[source..source_end]);
                }
                let tensor = float_tensor(spec.dtype, spec.shape.clone(), values)?;
                states.insert((*state_name).clone(), tensor);
            }
            let offset_tensor = Tensor::from_array((vec![1usize], vec![offset as i64]))
                .map_err(|error| PocketError::Runtime(error.to_string()))?
                .into_dyn();
            states.insert(state_offset, offset_tensor);
        }
        Ok(start_offset)
    }
}

fn load_session(path: &Path) -> Result<Session, PocketError> {
    let builder = Session::builder()
        .map_err(|error| PocketError::Runtime(format!("session builder: {error}")))?;
    let builder = builder
        .with_intra_threads(CPU_THREADS)
        .map_err(|error| PocketError::Runtime(format!("intra threads: {error}")))?;
    let mut builder = builder
        .with_inter_threads(1)
        .map_err(|error| PocketError::Runtime(format!("inter threads: {error}")))?;
    builder.commit_from_file(path).map_err(|error| {
        PocketError::Runtime(format!(
            "failed to load graph '{}': {error}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?")
        ))
    })
}

fn validate_main_graph(session: &Session) -> Result<(), PocketError> {
    for name in ["sequence", "text_embeddings"] {
        require_input(session, name, 3, "main graph")?;
    }
    for name in ["conditioning", "eos_logit"] {
        require_output(session, name, "main graph")?;
    }
    Ok(())
}

fn validate_flow_graph(session: &Session) -> Result<(), PocketError> {
    require_input(session, "c", 1, "flow graph")?;
    for name in ["s", "t", "x"] {
        require_input(session, name, 2, "flow graph")?;
    }
    require_output(session, "flow_dir", "flow graph")?;
    Ok(())
}

fn validate_decoder_graph(session: &Session) -> Result<(), PocketError> {
    require_input(session, "latent", 3, "decoder graph")?;
    require_output(session, "audio_frame", "decoder graph")?;
    Ok(())
}

fn validate_encoder_graph(session: &Session) -> Result<(), PocketError> {
    require_input(session, "audio", 3, "encoder graph")?;
    require_output(session, "conditioning", "encoder graph")?;
    Ok(())
}

fn validate_conditioner_graph(session: &Session) -> Result<(), PocketError> {
    require_input(session, "token_ids", 2, "text conditioner graph")?;
    require_output(session, "embeddings", "text conditioner graph")?;
    Ok(())
}

fn require_input(
    session: &Session,
    name: &str,
    rank: usize,
    graph: &str,
) -> Result<(), PocketError> {
    let input = session
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .ok_or_else(|| PocketError::Package(format!("{graph} has no input '{name}'")))?;
    match input.dtype() {
        ValueType::Tensor { shape, .. } if shape.len() == rank => Ok(()),
        ValueType::Tensor { shape, .. } => Err(PocketError::Package(format!(
            "{graph} input '{name}' has rank {}, expected {rank}",
            shape.len()
        ))),
        _ => Err(PocketError::Package(format!(
            "{graph} input '{name}' is not a tensor"
        ))),
    }
}

fn require_output(session: &Session, name: &str, graph: &str) -> Result<(), PocketError> {
    if session.outputs().iter().any(|output| output.name() == name) {
        Ok(())
    } else {
        Err(PocketError::Package(format!(
            "{graph} has no output '{name}'"
        )))
    }
}

type CollectedStates = (Vec<String>, HashMap<String, StateSpec>, usize);

fn collect_states(session: &Session, prefix: &str) -> Result<CollectedStates, PocketError> {
    let mut order = Vec::new();
    let mut specs = HashMap::new();
    let mut capacity = 0usize;
    for input in session.inputs() {
        if !input.name().starts_with(prefix) {
            continue;
        }
        let ValueType::Tensor { ty, shape, .. } = input.dtype() else {
            return Err(PocketError::Package(format!(
                "state '{}' is not a tensor",
                input.name()
            )));
        };
        if shape.iter().any(|dim| *dim < 0) {
            return Err(PocketError::Package(format!(
                "state '{}' has a dynamic shape {shape:?}",
                input.name()
            )));
        }
        if input.name() == "state_0" {
            if let Some(dim) = shape.get(1) {
                capacity = (*dim).max(0) as usize;
            }
        }
        order.push(input.name().to_string());
        specs.insert(
            input.name().to_string(),
            StateSpec {
                dtype: *ty,
                shape: shape.to_vec(),
            },
        );
    }
    Ok((order, specs, capacity))
}

fn initial_states(
    order: &[String],
    specs: &HashMap<String, StateSpec>,
) -> Result<HashMap<String, DynValue>, PocketError> {
    let mut states = HashMap::with_capacity(order.len());
    for name in order {
        let spec = specs
            .get(name)
            .ok_or_else(|| PocketError::Runtime(format!("missing state spec for {name}")))?;
        let count = spec
            .shape
            .iter()
            .map(|dim| *dim as usize)
            .product::<usize>();
        let value = match spec.dtype {
            TensorElementType::Float32 => Tensor::from_array((
                spec.shape
                    .iter()
                    .map(|dim| *dim as usize)
                    .collect::<Vec<_>>(),
                vec![0.0f32; count],
            ))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn(),
            TensorElementType::Float16 => Tensor::from_array((
                spec.shape
                    .iter()
                    .map(|dim| *dim as usize)
                    .collect::<Vec<_>>(),
                vec![f16::ZERO; count],
            ))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn(),
            TensorElementType::Int64 => Tensor::from_array((
                spec.shape
                    .iter()
                    .map(|dim| *dim as usize)
                    .collect::<Vec<_>>(),
                vec![0i64; count],
            ))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn(),
            TensorElementType::Bool => Tensor::from_array((
                spec.shape
                    .iter()
                    .map(|dim| *dim as usize)
                    .collect::<Vec<_>>(),
                vec![false; count],
            ))
            .map_err(|error| PocketError::Runtime(error.to_string()))?
            .into_dyn(),
            other => {
                return Err(PocketError::Runtime(format!(
                    "state {name} has unsupported dtype {other:?}"
                )))
            }
        };
        states.insert(name.clone(), value);
    }
    Ok(states)
}

fn float_tensor(
    dtype: TensorElementType,
    shape: Vec<i64>,
    values: Vec<f32>,
) -> Result<DynValue, PocketError> {
    let shape: Vec<usize> = shape.into_iter().map(|dim| dim.max(0) as usize).collect();
    match dtype {
        TensorElementType::Float32 => Tensor::from_array((shape, values))
            .map(|tensor| tensor.into_dyn())
            .map_err(|error| PocketError::Runtime(error.to_string())),
        TensorElementType::Float16 => Tensor::from_array((
            shape,
            values.into_iter().map(f16::from_f32).collect::<Vec<_>>(),
        ))
        .map(|tensor| tensor.into_dyn())
        .map_err(|error| PocketError::Runtime(error.to_string())),
        other => Err(PocketError::Runtime(format!(
            "voice state cache dtype {other:?} is unsupported"
        ))),
    }
}

fn load_bos(path: &Path) -> Result<Vec<f32>, PocketError> {
    let mut file = SafeTensorsFile::open(path).map_err(PocketError::Package)?;
    let (shape, values) = file
        .read_f32("flow_lm.bos_before_voice")
        .map_err(PocketError::Package)?;
    if shape.iter().product::<usize>() != values.len() || values.is_empty() {
        return Err(PocketError::Runtime(
            "flow_lm.bos_before_voice has an invalid shape".to_string(),
        ));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(PocketError::Runtime(
            "flow_lm.bos_before_voice contains non-finite values".to_string(),
        ));
    }
    Ok(values)
}

fn run_states(
    session: &mut Session,
    extra: Vec<(String, DynValue)>,
    states: &mut HashMap<String, DynValue>,
    state_order: &[String],
) -> Result<HashMap<String, DynValue>, PocketError> {
    let mut inputs: Vec<(String, ort::session::SessionInputValue<'_>)> = Vec::new();
    for (name, value) in extra {
        inputs.push((name, ort::session::SessionInputValue::Owned(value)));
    }
    for name in state_order {
        let state = states
            .get(name)
            .ok_or_else(|| PocketError::Runtime(format!("state '{name}' is not initialized")))?;
        inputs.push((
            name.clone(),
            ort::session::SessionInputValue::View(state.view()),
        ));
    }
    let outputs = session
        .run(inputs)
        .map_err(|error| PocketError::Runtime(format!("inference failed: {error}")))?;
    let mut returned = HashMap::new();
    let mut replacements = Vec::new();
    for (name, value) in outputs {
        if let Some(index) = name.strip_prefix("out_state_") {
            replacements.push((format!("state_{index}"), value));
        } else {
            returned.insert(name.to_string(), value);
        }
    }
    for (name, value) in replacements {
        states.insert(name, value);
    }
    Ok(returned)
}

fn extract_named_f32(
    outputs: &HashMap<String, DynValue>,
    name: &str,
) -> Result<Vec<f32>, PocketError> {
    extract_f32(outputs, name).map(|(_, values)| values)
}

fn extract_f32(
    outputs: &HashMap<String, DynValue>,
    name: &str,
) -> Result<(Vec<usize>, Vec<f32>), PocketError> {
    let value = outputs
        .get(name)
        .ok_or_else(|| PocketError::Runtime(format!("graph output '{name}' is missing")))?;
    let (shape, values) = value
        .try_extract_tensor::<f32>()
        .map_err(|error| PocketError::Runtime(format!("output '{name}' is not f32: {error}")))?;
    let shape = shape.iter().map(|dim| *dim as usize).collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(PocketError::Generation(format!(
            "output '{name}' contains non-finite values"
        )));
    }
    Ok((shape, values.to_vec()))
}

fn nan_sequence(latent_dim: usize) -> Result<DynValue, PocketError> {
    Tensor::from_array((vec![1usize, 1, latent_dim], vec![f32::NAN; latent_dim]))
        .map(|tensor| tensor.into_dyn())
        .map_err(|error| PocketError::Runtime(error.to_string()))
}

fn empty_text_embeddings(dim: usize) -> Result<DynValue, PocketError> {
    Tensor::from_array((vec![1usize, 0usize, dim], Vec::<f32>::new()))
        .map(|tensor| tensor.into_dyn())
        .map_err(|error| PocketError::Runtime(error.to_string()))
}

impl std::fmt::Debug for PocketRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PocketRuntime")
            .field("package", &self.descriptor.package_id)
            .field("quantized", &self.descriptor.graphs.quantized)
            .field("states", &self.main_state_order.len())
            .finish_non_exhaustive()
    }
}

struct NoiseRng {
    state: u64,
}

impl NoiseRng {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            state: time
                ^ ((std::process::id() as u64) << 32)
                ^ sequence.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        // SplitMix64: deterministic, dependency-free generator.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        let value = self.next_u64() >> 11;
        (value as f64 + 0.5) / ((1u64 << 53) as f64)
    }

    fn normal(&mut self, std: f32, count: usize) -> Vec<f32> {
        let mut result = Vec::with_capacity(count);
        while result.len() < count {
            let u1 = self.uniform();
            let u2 = self.uniform();
            let r = (-2.0 * u1.ln()).sqrt();
            let theta = 2.0 * std::f64::consts::PI * u2;
            result.push((r * theta.cos() * std as f64) as f32);
            if result.len() < count {
                result.push((r * theta.sin() * std as f64) as f32);
            }
        }
        result
    }
}

fn apply_fade_in(samples: &mut [f32], sample_rate: u32) {
    let fade = (sample_rate as usize / 200).max(1).min(samples.len());
    for (index, sample) in samples[..fade].iter_mut().enumerate() {
        *sample *= index as f32 / fade as f32;
    }
}

/// Softly compensate the synthesized level towards [`TARGET_OUTPUT_RMS`].
///
/// A reference normalized to `RMS ~0.048` makes the model noticeably quieter,
/// so quiet output is boosted up to [`MAX_OUTPUT_GAIN`] while the
/// [`OUTPUT_PEAK_LIMIT`] keeps the result below full scale. Corrections inside
/// [`OUTPUT_GAIN_DEADBAND`] are skipped. Returns the applied gain (`1.0` when
/// the buffer was left untouched).
fn compensate_output_gain(samples: &mut [f32]) -> f32 {
    if samples.is_empty() {
        return 1.0;
    }
    let rms = (samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32;
    if !rms.is_finite() || rms <= 0.0 {
        return 1.0;
    }
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0f32, f32::max);
    let gain = (TARGET_OUTPUT_RMS / rms)
        .min(OUTPUT_PEAK_LIMIT / peak.max(1e-6))
        .min(MAX_OUTPUT_GAIN);
    if !gain.is_finite() || (gain - 1.0).abs() <= OUTPUT_GAIN_DEADBAND {
        return 1.0;
    }
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
    gain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rms: f32, sample_rate: u32, hz: f64) -> Vec<f32> {
        let amplitude = rms as f64 * std::f64::consts::SQRT_2;
        (0..sample_rate)
            .map(|index| {
                let phase = 2.0 * std::f64::consts::PI * hz * index as f64 / sample_rate as f64;
                (phase.sin() * amplitude) as f32
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt() as f32
    }

    #[test]
    fn output_gain_boosts_quiet_speech_without_clipping() {
        let mut samples = sine(0.04, 24_000, 180.0);
        let gain = compensate_output_gain(&mut samples);

        assert!(
            gain > 1.0,
            "quiet output must be boosted, got gain {gain:.4}"
        );
        assert!(
            gain <= MAX_OUTPUT_GAIN,
            "gain {gain:.4} exceeds the {MAX_OUTPUT_GAIN} cap"
        );
        let peak = samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);
        assert!(peak <= 0.95, "compensated peak {peak:.4} clips");
        let level = rms(&samples);
        assert!(
            (level - TARGET_OUTPUT_RMS).abs() < 0.01,
            "compensated RMS {level:.4} is not near {TARGET_OUTPUT_RMS}"
        );
    }

    #[test]
    fn output_gain_leaves_already_loud_speech_nearly_untouched() {
        let mut samples = sine(0.4, 24_000, 180.0);
        let before = samples.clone();
        let gain = compensate_output_gain(&mut samples);
        assert!(
            gain <= 1.0,
            "loud output must not be boosted, got {gain:.4}"
        );
        let peak = samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);
        assert!(peak <= 0.95, "compensated peak {peak:.4} clips");
        // The correction is either inaudible (deadband) or a real attenuation.
        assert!(gain < 1.0 || samples == before);
    }
}
