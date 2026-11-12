//! Native integration test for the Pocket TTS runtime.
//!
//! Ignored by default because it needs real model packages. Stage the sets
//! under `TTSBARD_POCKET_TEST_ROOT/models/pocket` and run:
//!
//! ```text
//! cargo test pocket_native_ru_en_ru_roundtrip -- --ignored --nocapture
//! ```
//!
//! Optional variables:
//! * `TTSBARD_POCKET_TEST_RU_WAV` — reference WAV for the Russian set;
//! * `TTSBARD_POCKET_TEST_EN_WAV` — reference WAV for an English cloning set;
//! * `TTSBARD_POCKET_TEST_OUT` — directory for generated WAV files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::descriptor::scan_pocket_models_dir;
use super::provider::PocketTts;
use super::runtime::PocketRuntime;
use super::tokenizer::PocketTokenizer;
use super::voice::PocketVoiceSelection;
use super::PocketError;
use crate::tts::engine::TtsEngine;

fn env_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a path"))
}

fn write_output(out_dir: Option<&Path>, name: &str, wav: &[u8]) {
    if let Some(out_dir) = out_dir {
        let _ = std::fs::create_dir_all(out_dir);
        let path = out_dir.join(name);
        if let Err(error) = std::fs::write(&path, wav) {
            eprintln!("failed to write {}: {error}", path.display());
        } else {
            eprintln!("wrote {}", path.display());
        }
    }
}

fn check_wav(wav: &[u8], label: &str) -> f32 {
    assert!(!wav.is_empty(), "{label}: WAV is empty");
    let pcm = crate::audio::effects::decode_audio(wav)
        .unwrap_or_else(|error| panic!("{label}: output is not decodable WAV: {error}"));
    assert_eq!(pcm.sample_rate, 24000, "{label}: unexpected sample rate");
    assert_eq!(pcm.channels, 1, "{label}: expected mono");
    assert!(pcm.frame_count() > 0, "{label}: no audio frames");
    let rms = (pcm
        .samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / pcm.samples.len() as f64)
        .sqrt();
    assert!(rms > 0.001, "{label}: output is silent (rms={rms})");
    eprintln!(
        "{label}: {} frames, {:.2}s, rms={rms:.4}",
        pcm.frame_count(),
        pcm.duration_secs()
    );
    rms as f32
}

fn synthesize_checked(
    runtime: &mut PocketRuntime,
    text: &str,
    voice: &PocketVoiceSelection,
    label: &str,
    out_dir: Option<&Path>,
) {
    let cancel = AtomicBool::new(false);
    let output = runtime
        .synthesize(text, Some(voice), &cancel)
        .unwrap_or_else(|error| panic!("{label}: synthesis failed: {error:?}"));
    assert!(
        output.stats.eos_frame.is_some(),
        "{label}: generation returned without EOS"
    );
    assert!(
        output.stats.frames > 0,
        "{label}: generation produced no latent frames"
    );
    assert!(
        output.stats.duration_seconds > 0.5,
        "{label}: generated clip is too short"
    );
    check_wav(&output.wav, label);
    write_output(out_dir, &format!("{label}.wav"), &output.wav);
    eprintln!(
        "{label}: EOS at frame {:?}, {} frames, {:.2}s",
        output.stats.eos_frame, output.stats.frames, output.stats.duration_seconds
    );
}

#[test]
#[ignore = "set TTSBARD_POCKET_TEST_ROOT to a staged models/pocket directory; native ONNX inference"]
fn pocket_native_ru_en_ru_roundtrip() {
    let root = env_path("TTSBARD_POCKET_TEST_ROOT");
    let reference = env_path("TTSBARD_POCKET_TEST_RU_WAV");
    let out_dir = std::env::var_os("TTSBARD_POCKET_TEST_OUT").map(PathBuf::from);
    assert!(
        reference.is_file(),
        "reference WAV does not exist: {}",
        reference.display()
    );

    let descriptors = scan_pocket_models_dir(&root.join("models/pocket"));
    assert!(
        descriptors.len() >= 2,
        "expected RU and EN packages under {}, found {}",
        root.display(),
        descriptors.len()
    );
    let ru = descriptors
        .iter()
        .find(|descriptor| descriptor.language == "ru")
        .expect("Russian package is missing");
    let en = descriptors
        .iter()
        .find(|descriptor| descriptor.language == "en")
        .expect("English package is missing");
    eprintln!(
        "packages: {} ({:?}), {} ({:?})",
        ru.id, ru.graphs.quantized, en.id, en.graphs.quantized
    );

    let ru_voice = PocketVoiceSelection::ReferenceWav(reference.clone());
    let en_voice = if let Some(path) = std::env::var_os("TTSBARD_POCKET_TEST_EN_WAV") {
        PocketVoiceSelection::ReferenceWav(PathBuf::from(path))
    } else {
        PocketVoiceSelection::VoiceState(
            en.voices
                .first()
                .expect("English package has no ready voice; set TTSBARD_POCKET_TEST_EN_WAV")
                .id
                .clone(),
        )
    };

    // The two tokenizer formats must reproduce the reference Python ids.
    let ru_tokenizer = PocketTokenizer::sentencepiece(&ru.tokenizer_path, ru.n_bins)
        .expect("Shtorm SentencePiece tokenizer must load");
    let ru_text =
        "За окно\u{0301}м шуми\u{0301}т дождь, а в ко\u{0301}мнате ти\u{0301}кают часы\u{0301}.";
    assert_eq!(
        ru_tokenizer.encode(ru_text).unwrap(),
        vec![
            482, 1017, 1479, 203, 3908, 528, 3901, 101, 3921, 343, 3911, 109, 10, 3275, 1262, 51,
            410, 1162, 34, 3914
        ],
        "Shtorm SentencePiece ids differ from the reference"
    );
    let en_tokenizer = PocketTokenizer::json(&en.tokenizer_path, en.n_bins)
        .expect("Kyutai tokenizer.json must load");
    assert_eq!(
        en_tokenizer
            .encode("Rain is falling outside the window.")
            .unwrap(),
        vec![1213, 356, 277, 1167, 273, 321, 677, 265, 260, 2330, 263],
        "Kyutai tokenizer.json ids differ from the reference"
    );

    // RU -> EN -> RU on separate runtimes, matching the user-visible switch.
    for _ in 0..2 {
        let cancel = AtomicBool::new(false);
        let mut runtime = PocketRuntime::load(ru, &cancel).expect("RU runtime must load");
        synthesize_checked(
            &mut runtime,
            "За окно\u{0301}м шуми\u{0301}т дождь, а в ко\u{0301}мнате ти\u{0301}кают часы\u{0301}.",
            &ru_voice,
            "ru",
            out_dir.as_deref(),
        );
        // Missing reference must fail with the dedicated code without
        // poisoning the runtime or the follow-up call below.
        let missing = PocketVoiceSelection::ReferenceWav(root.join("missing-reference.wav"));
        let err = runtime
            .synthesize("Проверка отсутствующего файла.", Some(&missing), &cancel)
            .expect_err("missing reference must fail");
        assert!(
            matches!(err, PocketError::ReferenceMissing),
            "unexpected missing-reference error: {err:?}"
        );
        synthesize_checked(
            &mut runtime,
            "После ошибки модель снова готова к работе.",
            &ru_voice,
            "ru-after-missing",
            out_dir.as_deref(),
        );

        let cancel = AtomicBool::new(false);
        let mut runtime = PocketRuntime::load(en, &cancel).expect("EN runtime must load");
        synthesize_checked(
            &mut runtime,
            "Rain is falling outside the window.",
            &en_voice,
            "en",
            out_dir.as_deref(),
        );
    }

    // Cancellation through the provider used by the queue: the worker stops at
    // a frame boundary and the next request can reuse the same provider.
    let provider = PocketTts::new(ru.to_owned(), Some(ru_voice.clone()));
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = provider.clone();
    let worker_cancel = Arc::clone(&cancel);
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(worker.synthesize_cancellable(
            "Отмена должна остановить длинную генерацию и не блокировать следующую задачу.",
            worker_cancel,
        ))
    });
    std::thread::sleep(Duration::from_millis(300));
    cancel.store(true, Ordering::Relaxed);
    let cancelled = handle.join().expect("cancellation worker must not panic");
    assert!(
        cancelled.is_err(),
        "cancelled synthesis unexpectedly completed successfully"
    );
    eprintln!("cancelled synthesis returned: {cancelled:?}");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let after_cancel = runtime
        .block_on(provider.synthesize("Следующая задача после отмены."))
        .expect("the next synthesis after cancellation must succeed");
    check_wav(&after_cancel, "ru-after-cancel");
    write_output(out_dir.as_deref(), "ru-after-cancel.wav", &after_cancel);

    eprintln!("Pocket TTS native RU -> EN -> RU roundtrip passed");
}
