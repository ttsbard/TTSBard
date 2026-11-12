//! Reference voice preparation and PCM output helpers for Pocket TTS.
//!
//! The model expects mono 24 kHz audio. A user-supplied reference is decoded,
//! mixed to mono, resampled with a windowed-sinc polyphase filter, ended on an
//! 80 ms pause (matching the Python reference pipeline), and padded to a whole
//! Mimi frame.

use std::path::Path;

use super::PocketError;

/// Maximum reference file size accepted by the validator.
const MAX_REFERENCE_BYTES: u64 = 200 * 1024 * 1024;
/// Minimum reference duration accepted by the validator.
const MIN_REFERENCE_SECONDS: f32 = 0.05;
/// Bound the peak memory of the ONNX voice encoder.
const MAX_REFERENCE_SECONDS: u32 = 12;
/// Silence threshold for rejecting an empty recording.
const MIN_REFERENCE_RMS: f32 = 0.0005;
/// Target RMS of the active speech after reference normalization (ROADMAP-137).
///
/// Mimi and FlowLM are not amplitude invariant: the reference level that keeps
/// the original F0 on both female and male voices is about `0.048`.
const TARGET_REFERENCE_RMS: f32 = 0.048;
/// A normalized reference must never clip; keep the peak below full scale.
const REFERENCE_PEAK_LIMIT: f32 = 0.98;
/// Analysis frame for the active-speech level: 20 ms at the 24 kHz target.
const REFERENCE_ANALYSIS_FRAME: usize = 480;
/// Sample rate every prepared Pocket TTS reference is stored at.
const REFERENCE_SAMPLE_RATE: u32 = 24_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceAudioInfo {
    pub file_name: String,
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub channels: usize,
}

/// Validate a user-selected reference WAV.
///
/// Missing and unreadable files are distinct errors so the UI can say either
/// "file disappeared, choose it again" or "this WAV is invalid".
pub fn validate_reference_wav(path: &Path) -> Result<ReferenceAudioInfo, PocketError> {
    let audio = decode_reference(path)?;
    let duration_seconds =
        audio.samples.len() as f32 / (audio.sample_rate as f32 * audio.channels as f32);
    let rms = (audio
        .samples
        .iter()
        .map(|sample| sample * sample)
        .sum::<f32>()
        / audio.samples.len().max(1) as f32)
        .sqrt();
    if !rms.is_finite() || rms < MIN_REFERENCE_RMS {
        return Err(PocketError::ReferenceInvalid(
            "reference is silent".to_string(),
        ));
    }
    Ok(ReferenceAudioInfo {
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("reference.wav")
            .to_string(),
        duration_seconds,
        sample_rate: audio.sample_rate,
        channels: audio.channels,
    })
}

/// Load and prepare a reference for Mimi: mono, 24 kHz, ended on a pause and
/// padded to a whole Mimi frame.
pub fn prepare_reference_wav(
    path: &Path,
    target_sample_rate: u32,
    frame_samples: usize,
) -> Result<Vec<f32>, PocketError> {
    let audio = decode_reference(path)?;
    if audio.samples.is_empty() {
        return Err(PocketError::ReferenceInvalid(
            "reference contains no samples".to_string(),
        ));
    }
    let mono = to_mono(&audio.samples, audio.channels);
    let mut resampled = resample(&mono, audio.sample_rate, target_sample_rate);
    if resampled.is_empty() {
        return Err(PocketError::ReferenceInvalid(
            "reference is too short after resampling".to_string(),
        ));
    }
    resampled = end_on_pause(&resampled, target_sample_rate);
    if frame_samples > 1 {
        let remainder = resampled.len() % frame_samples;
        if remainder != 0 {
            resampled.resize(resampled.len() + frame_samples - remainder, 0.0);
        }
    }
    Ok(resampled)
}

/// Prepare a normalized copy of a user reference for Pocket TTS.
///
/// The source file is never modified: it is decoded, mixed to mono, resampled
/// to 24 kHz, scaled so the active speech sits at [`TARGET_REFERENCE_RMS`], and
/// written to `dest` (atomically, creating the parent directory). The returned
/// info describes the written file, so callers can persist the new path.
pub fn normalize_reference_wav_to_file(
    source: &Path,
    dest: &Path,
) -> Result<ReferenceAudioInfo, PocketError> {
    let audio = decode_reference(source)?;
    let mono = to_mono(&audio.samples, audio.channels);
    let resampled = resample(&mono, audio.sample_rate, REFERENCE_SAMPLE_RATE);
    if resampled.is_empty() {
        return Err(PocketError::ReferenceInvalid(
            "reference is too short after resampling".to_string(),
        ));
    }
    let normalized = normalize_reference_level(&resampled);
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                PocketError::Runtime(format!(
                    "cannot create the normalized reference directory: {error}"
                ))
            })?;
        }
    }
    let bytes = crate::audio::effects::encode_wav(&normalized, REFERENCE_SAMPLE_RATE, 1)
        .map_err(|error| {
            PocketError::Runtime(format!("cannot encode the normalized reference: {error}"))
        })?;
    crate::config::replace_file_atomically(dest, &bytes).map_err(|error| {
        PocketError::Runtime(format!("cannot write the normalized reference: {error}"))
    })?;
    validate_reference_wav(dest)
}

/// Scale `samples` so the active speech RMS equals [`TARGET_REFERENCE_RMS`].
///
/// Frames within 20 dB of the overall level (`>= 0.1 * overall RMS`) count as
/// active speech; a fully silent or unanalyzable signal falls back to the
/// overall RMS. The gain is capped at `0.98 / peak` so the result never clips.
fn normalize_reference_level(samples: &[f32]) -> Vec<f32> {
    let overall = reference_rms(samples);
    if !overall.is_finite() || overall <= 0.0 {
        return samples.to_vec();
    }
    let active = active_speech_rms(samples, overall).unwrap_or(overall);
    if !active.is_finite() || active <= 0.0 {
        return samples.to_vec();
    }
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return samples.to_vec();
    }
    let gain = (TARGET_REFERENCE_RMS / active).min(REFERENCE_PEAK_LIMIT / peak);
    if !gain.is_finite() || gain <= 0.0 {
        return samples.to_vec();
    }
    samples
        .iter()
        .map(|sample| (sample * gain).clamp(-REFERENCE_PEAK_LIMIT, REFERENCE_PEAK_LIMIT))
        .collect()
}

/// RMS over whole 20 ms frames whose level is at least a tenth of `overall`.
fn active_speech_rms(samples: &[f32], overall: f32) -> Option<f32> {
    if samples.len() < REFERENCE_ANALYSIS_FRAME {
        return None;
    }
    let threshold = 0.1 * overall;
    if !threshold.is_finite() || threshold <= 0.0 {
        return None;
    }
    let threshold = threshold as f64;
    let mut energy = 0.0f64;
    let mut frames = 0usize;
    for frame in samples.chunks_exact(REFERENCE_ANALYSIS_FRAME) {
        let frame_energy = frame
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / frame.len() as f64;
        if frame_energy.sqrt() >= threshold {
            energy += frame_energy;
            frames += 1;
        }
    }
    if frames == 0 {
        None
    } else {
        Some((energy / frames as f64).sqrt() as f32)
    }
}

/// Root-mean-square level of a block of samples.
fn reference_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32
}

fn decode_reference(path: &Path) -> Result<crate::audio::effects::AudioPcm, PocketError> {
    if !path.exists() {
        return Err(PocketError::ReferenceMissing);
    }
    if !path.is_file() {
        return Err(PocketError::ReferenceInvalid(
            "reference is not a file".to_string(),
        ));
    }
    let is_wav = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);
    if !is_wav {
        return Err(PocketError::ReferenceInvalid(
            "reference must be a .wav file".to_string(),
        ));
    }
    let metadata = std::fs::metadata(path).map_err(|error| {
        PocketError::ReferenceInvalid(format!("cannot read reference metadata: {error}"))
    })?;
    if metadata.len() == 0 {
        return Err(PocketError::ReferenceInvalid(
            "reference file is empty".to_string(),
        ));
    }
    if metadata.len() > MAX_REFERENCE_BYTES {
        return Err(PocketError::ReferenceInvalid(
            "reference file is too large".to_string(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        PocketError::ReferenceInvalid(format!("cannot read reference: {error}"))
    })?;
    let audio = crate::audio::effects::decode_audio(&bytes).map_err(|error| {
        PocketError::ReferenceInvalid(format!("reference is not decodable: {error}"))
    })?;
    if audio.sample_rate == 0 || audio.channels == 0 || audio.samples.is_empty() {
        return Err(PocketError::ReferenceInvalid(
            "reference has no audio samples".to_string(),
        ));
    }
    let duration = audio.samples.len() as f32 / (audio.sample_rate as f32 * audio.channels as f32);
    if duration < MIN_REFERENCE_SECONDS {
        return Err(PocketError::ReferenceInvalid(
            "reference is too short".to_string(),
        ));
    }
    if duration > MAX_REFERENCE_SECONDS as f32 {
        return Err(PocketError::ReferenceTooLong {
            max_seconds: MAX_REFERENCE_SECONDS,
        });
    }
    if audio.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(PocketError::ReferenceInvalid(
            "reference contains non-finite samples".to_string(),
        ));
    }
    Ok(audio)
}

fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let frames = samples.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut sum = 0.0f32;
        for channel in 0..channels {
            sum += samples[frame * channels + channel];
        }
        mono.push(sum / channels as f32);
    }
    mono
}

/// Rational polyphase resampler with a Hann-windowed sinc kernel.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return input.to_vec();
    }
    let gcd = gcd_u32(from_rate, to_rate);
    let up = (to_rate / gcd) as usize;
    let down = (from_rate / gcd) as usize;
    let output_len = (input.len() as u128 * up as u128).div_ceil(down as u128) as usize;
    let cutoff = (up as f64 / down as f64).min(1.0);
    // 24 taps per side is a good compromise for short voice references while
    // keeping the 44.1 -> 24 kHz conversion fast on the blocking worker.
    const HALF_TAPS: isize = 24;

    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        let numerator = output_index as u128 * down as u128;
        let base = (numerator / up as u128) as isize;
        let phase = (numerator % up as u128) as f64 / up as f64;
        let mut sum = 0.0f64;
        let mut weight_sum = 0.0f64;
        for tap in (-HALF_TAPS + 1)..=HALF_TAPS {
            let input_index = base + tap;
            if input_index < 0 || input_index as usize >= input.len() {
                continue;
            }
            let distance = phase - tap as f64;
            let window_position =
                (distance + HALF_TAPS as f64) / (2.0 * HALF_TAPS as f64) * std::f64::consts::PI;
            let window = if window_position <= 0.0 || window_position >= std::f64::consts::PI {
                0.0
            } else {
                0.5 - 0.5 * window_position.cos()
            };
            let x = std::f64::consts::PI * cutoff * distance;
            let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
            let weight = cutoff * sinc * window;
            sum += input[input_index as usize] as f64 * weight;
            weight_sum += weight;
        }
        if weight_sum.abs() > 1e-12 {
            output.push((sum / weight_sum) as f32);
        } else {
            output.push(0.0);
        }
    }
    output
}

fn gcd_u32(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

/// End the prompt on exactly 80 ms of silence:
/// 1. find the last 20 ms frame within 35 dB of the loudest one;
/// 2. cut everything after it;
/// 3. fade the last 20 ms out;
/// 4. append 80 ms of zeros.
pub fn end_on_pause(input: &[f32], sample_rate: u32) -> Vec<f32> {
    if input.is_empty() || sample_rate == 0 {
        return input.to_vec();
    }
    let frame = ((0.02 * sample_rate as f32).round() as usize).max(1);
    let frames = input.len() / frame;
    if frames == 0 {
        return input.to_vec();
    }
    let mut rms_values = Vec::with_capacity(frames);
    for index in 0..frames {
        let start = index * frame;
        let end = start + frame;
        let energy = input[start..end]
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / frame as f64;
        rms_values.push(energy.sqrt());
    }
    let max_rms = rms_values.iter().cloned().fold(0.0f64, f64::max);
    let threshold = max_rms * 10f64.powf(-35.0 / 20.0);
    let last_loud = rms_values
        .iter()
        .rposition(|rms| *rms > threshold)
        .unwrap_or(0);
    let end = ((last_loud + 1) * frame).min(input.len());
    let mut result = input[..end].to_vec();

    let fade = ((0.02 * sample_rate as f32).round() as usize).min(result.len());
    if fade > 1 {
        for (index, sample) in result[end - fade..].iter_mut().enumerate() {
            let gain = 1.0 - (index as f32 / (fade - 1) as f32);
            *sample *= gain;
        }
    }
    let pause = ((0.08 * sample_rate as f32).round() as usize).max(1);
    result.resize(result.len() + pause, 0.0);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::effects::encode_wav;

    fn temp_wav(
        name: &str,
        samples: &[f32],
        sample_rate: u32,
        channels: usize,
    ) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ttsbard-pocket-audio-{}-{}-{name}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = encode_wav(samples, sample_rate, channels).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A unique path under the system temp directory for one test artifact.
    fn temp_artifact(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "ttsbard-pocket-audio-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn missing_reference_is_distinct_from_invalid() {
        let missing = std::env::temp_dir().join("ttsbard-pocket-does-not-exist.wav");
        assert!(matches!(
            validate_reference_wav(&missing),
            Err(PocketError::ReferenceMissing)
        ));

        let path =
            std::env::temp_dir().join(format!("ttsbard-pocket-invalid-{}.wav", std::process::id()));
        std::fs::write(&path, b"not a wav").unwrap();
        let result = validate_reference_wav(&path);
        assert!(matches!(result, Err(PocketError::ReferenceInvalid(_))));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn validates_a_short_reference() {
        let samples: Vec<f32> = (0..24000)
            .map(|index| (index as f32 * 0.05).sin() * 0.2)
            .collect();
        let path = temp_wav("valid", &samples, 24000, 1);
        let info = validate_reference_wav(&path).unwrap();
        assert!(info.duration_seconds > 0.9);
        assert_eq!(info.sample_rate, 24000);
        assert_eq!(info.channels, 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn reference_duration_limit_is_inclusive() {
        let sample_rate = 24_000;
        let samples = vec![0.2; (sample_rate * MAX_REFERENCE_SECONDS + 1) as usize];
        let at_limit = temp_wav("at-limit", &samples[..samples.len() - 1], sample_rate, 1);
        let too_long = temp_wav("too-long", &samples, sample_rate, 1);
        assert!(validate_reference_wav(&at_limit).is_ok());
        assert!(matches!(
            validate_reference_wav(&too_long),
            Err(PocketError::ReferenceTooLong {
                max_seconds: MAX_REFERENCE_SECONDS
            })
        ));
        std::fs::remove_file(at_limit).ok();
        std::fs::remove_file(too_long).ok();
    }

    #[test]
    fn silent_reference_is_rejected() {
        let path = temp_wav("silent", &vec![0.0; 24000], 24000, 1);
        assert!(matches!(
            validate_reference_wav(&path),
            Err(PocketError::ReferenceInvalid(_))
        ));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn resamples_44100_to_24000_with_expected_length() {
        let input: Vec<f32> = (0..44100)
            .map(|index| (index as f32 * 0.03).sin() * 0.3)
            .collect();
        let output = resample(&input, 44100, 24000);
        assert_eq!(output.len(), 24000);
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(output.iter().any(|sample| sample.abs() > 0.1));
    }

    /// Root-mean-square level of a block of samples.
    fn rms(samples: &[f32]) -> f64 {
        (samples
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt()
    }

    /// A 9 kHz tone is below the 12 kHz Nyquist of the 24 kHz target, so the
    /// anti-aliasing low-pass must pass it almost unchanged. The old cutoff
    /// (`0.5 *` of the resampling ratio) put the corner near 6 kHz and would
    /// attenuate this tone by far more than the 0.7 threshold below.
    #[test]
    fn resampling_44100_to_24000_preserves_9khz_tone() {
        let from_rate = 44_100u32;
        let to_rate = 24_000u32;
        let tone_hz = 9_000.0f64;
        let amplitude = 0.5f64;
        let input: Vec<f32> = (0..from_rate)
            .map(|index| {
                let phase = 2.0 * std::f64::consts::PI * tone_hz * index as f64 / from_rate as f64;
                (phase.sin() * amplitude) as f32
            })
            .collect();
        let output = resample(&input, from_rate, to_rate);
        assert_eq!(output.len(), 24_000);

        // Skip the filter's edge transients: compare the steady-state middle.
        let skip = output.len() / 10;
        let output_rms = rms(&output[skip..output.len() - skip]);
        let input_rms = rms(&input[skip..input.len() - skip]);
        assert!(
            output_rms >= 0.7 * input_rms,
            "9 kHz tone lost too much energy: input RMS {input_rms:.4}, output RMS {output_rms:.4}"
        );
    }

    #[test]
    fn end_on_pause_trims_trailing_silence_and_appends_80ms() {
        let mut input = vec![0.2f32; 24000];
        input.extend(std::iter::repeat_n(0.0, 24000));
        let output = end_on_pause(&input, 24000);
        // Last loud frame is trimmed, then 80 ms of silence are added.
        assert!(output.len() >= 24000);
        assert!(output.len() <= 24000 + 24000 + 1920);
        let tail = &output[output.len() - 480..];
        assert!(tail.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn prepare_reference_pads_to_mimi_frame() {
        let samples: Vec<f32> = (0..44100)
            .map(|index| (index as f32 * 0.02).sin() * 0.2)
            .collect();
        let path = temp_wav("prepare", &samples, 44100, 1);
        let prepared = prepare_reference_wav(&path, 24000, 1920).unwrap();
        assert!(!prepared.is_empty());
        assert_eq!(prepared.len() % 1920, 0);
        std::fs::remove_file(path).ok();
    }

    /// A loud 44.1 kHz stereo reference (RMS ~0.20) must be stored as mono
    /// 24 kHz with the active level near the 0.048 target.
    #[test]
    fn normalizes_loud_stereo_reference_to_target_rms() {
        let source_rate = 44_100u32;
        let amplitude = 0.20f64 * std::f64::consts::SQRT_2;
        let mut stereo = Vec::with_capacity(source_rate as usize * 2);
        for index in 0..source_rate {
            let phase = 2.0 * std::f64::consts::PI * 220.0 * index as f64 / source_rate as f64;
            let value = (phase.sin() * amplitude) as f32;
            stereo.push(value);
            stereo.push(value);
        }
        assert!((rms(&stereo) - 0.20).abs() < 0.01);

        let source = temp_wav("normalize-loud-source", &stereo, source_rate, 2);
        let destination_dir = temp_artifact("normalize-loud-dir");
        let dest = destination_dir.join("nested").join("reference.wav");

        let info = normalize_reference_wav_to_file(&source, &dest).unwrap();
        assert_eq!(info.sample_rate, 24_000);
        assert_eq!(info.channels, 1);

        let written = crate::audio::effects::decode_audio(&std::fs::read(&dest).unwrap()).unwrap();
        let level = rms(&written.samples);
        assert!(
            (0.040..=0.056).contains(&level),
            "normalized RMS {level:.4} is outside 0.040..0.056"
        );
        let peak = written
            .samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);
        assert!(peak <= 0.98, "normalized peak {peak:.4} clips");

        std::fs::remove_file(source).ok();
        std::fs::remove_dir_all(destination_dir).ok();
    }

    /// A quiet reference (RMS ~0.015) must be scaled up to the target level
    /// without clipping.
    #[test]
    fn normalizes_quiet_reference_without_clipping() {
        let sample_rate = 24_000u32;
        let amplitude = 0.015f64 * std::f64::consts::SQRT_2;
        let samples: Vec<f32> = (0..sample_rate)
            .map(|index| {
                let phase =
                    2.0 * std::f64::consts::PI * 200.0 * index as f64 / sample_rate as f64;
                (phase.sin() * amplitude) as f32
            })
            .collect();
        assert!((rms(&samples) - 0.015).abs() < 0.001);

        let source = temp_wav("normalize-quiet-source", &samples, sample_rate, 1);
        let destination_dir = temp_artifact("normalize-quiet-dir");
        let dest = destination_dir.join("reference.wav");

        normalize_reference_wav_to_file(&source, &dest).unwrap();

        let written = crate::audio::effects::decode_audio(&std::fs::read(&dest).unwrap()).unwrap();
        let peak = written
            .samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max);
        let level = rms(&written.samples);
        assert!(peak <= 0.98, "normalized peak {peak:.4} clips");
        assert!(
            (0.040..=0.056).contains(&level),
            "quiet reference was not scaled to the target: RMS {level:.4}"
        );

        std::fs::remove_file(source).ok();
        std::fs::remove_dir_all(destination_dir).ok();
    }
}
