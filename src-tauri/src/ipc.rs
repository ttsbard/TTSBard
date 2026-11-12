use serde::Serialize;
use std::fmt;

/// Stable IPC names for the speech submission vertical slice.
pub mod speech {
    pub const SUBMIT_COMMAND: &str = "submit_speech";
    pub const QUEUE_CHANGED_EVENT: &str = "speech-queue-changed";

    pub mod error_code {
        pub const SNAPSHOT_UNAVAILABLE: &str = "speech.snapshot_unavailable";
        pub const EMPTY_TEXT: &str = "speech.empty_text";
        pub const QUEUE_FULL: &str = "speech.queue_full";
        pub const QUEUE_REJECTED: &str = "speech.queue_rejected";
        pub const TWITCH_ONLY_ROUTE: &str = "speech.twitch_only_route";
    }
}

/// Stable IPC names for the Twitch-only delivery vertical slice.
pub mod twitch_delivery {
    pub const DELIVER_COMMAND: &str = "deliver_twitch_message";
    pub const DELIVERY_FAILED_EVENT: &str = "twitch-delivery-failed";

    pub mod error_code {
        pub const EMPTY_TEXT: &str = "twitch.empty_text";
        pub const UNAVAILABLE: &str = "twitch.unavailable";
        pub const SEND_FAILED: &str = "twitch.send_failed";
        pub const DELIVERY_UNKNOWN: &str = "twitch.delivery_unknown";
        pub const QUEUE_FULL: &str = "twitch.queue_full";
        pub const TOO_LONG: &str = "twitch.too_long";
        pub const PARTIAL_DELIVERY: &str = "twitch.partial_delivery";
    }
}

/// Stable error codes for TTS provider management commands (ROADMAP-119).
pub mod tts_provider {
    pub mod error_code {
        /// The requested provider ID is not registered.
        pub const UNKNOWN_ID: &str = "tts_provider.unknown_id";
        /// The eSpeak readiness check could not be evaluated.
        pub const READINESS_CHECK_FAILED: &str = "tts_provider.readiness_check_failed";
        /// eSpeak NG data is missing; Piper loading is blocked until it is
        /// restored and the application is restarted.
        pub const ESPEAK_DATA_NOT_FOUND: &str = "tts_provider.espeak_data_not_found";
        /// A visible espeak-ng-data folder failed the phonemization probe.
        pub const ESPEAK_DATA_DAMAGED: &str = "tts_provider.espeak_data_damaged";
        pub const ESPEAK_VOICE_UNAVAILABLE: &str = "tts_provider.espeak_voice_unavailable";
        /// The prepare/selection worker task failed.
        pub const PREPARE_TASK_FAILED: &str = "tts_provider.prepare_task_failed";
        /// The selection could not be persisted.
        pub const SELECTION_FAILED: &str = "tts_provider.selection_failed";
    }
}

/// Stable error/status codes for the Pocket TTS discovery and voice contract.
pub mod pocket {
    pub mod error_code {
        pub const UNKNOWN_MODEL: &str = "pocket.unknown_model";
        pub const PACKAGE_INVALID: &str = "pocket.package_invalid";
        pub const REFERENCE_REQUIRED: &str = "pocket.reference_required";
        pub const REFERENCE_MISSING: &str = "pocket.reference_missing";
        pub const REFERENCE_INVALID: &str = "pocket.reference_invalid";
        pub const VOICE_MISSING: &str = "pocket.voice_missing";
        pub const RUNTIME_FAILED: &str = "pocket.runtime_failed";
        pub const GENERATION_FAILED: &str = "pocket.generation_failed";
        pub const CANCELLED: &str = "pocket.cancelled";
        pub const PERSISTENCE_FAILED: &str = "pocket.persistence_failed";
    }

    pub mod readiness {
        pub const READY: &str = "ready";
        pub const NEEDS_REFERENCE: &str = "needs_reference";
        pub const REFERENCE_MISSING: &str = "reference_missing";
        pub const REFERENCE_INVALID: &str = "reference_invalid";
        pub const VOICE_MISSING: &str = "voice_missing";
        pub const PACKAGE_INVALID: &str = "package_invalid";
    }
}

/// Exhaustive production declaration: every `submit_speech` error code and its retryability.
#[derive(Debug, Clone, Serialize)]
pub struct SpeechErrorDef {
    pub code: &'static str,
    pub retryable: bool,
}

pub const SPEECH_SUBMIT_ERRORS: &[SpeechErrorDef] = &[
    SpeechErrorDef {
        code: speech::error_code::SNAPSHOT_UNAVAILABLE,
        retryable: false,
    },
    SpeechErrorDef {
        code: speech::error_code::EMPTY_TEXT,
        retryable: false,
    },
    SpeechErrorDef {
        code: speech::error_code::QUEUE_FULL,
        retryable: true,
    },
    SpeechErrorDef {
        code: speech::error_code::QUEUE_REJECTED,
        retryable: false,
    },
    SpeechErrorDef {
        code: speech::error_code::TWITCH_ONLY_ROUTE,
        retryable: false,
    },
];

pub fn speech_error_code_to_retryable(code: &str) -> bool {
    SPEECH_SUBMIT_ERRORS
        .iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("unknown submit_speech error code: {code}"))
        .retryable
}

/// Exhaustive production declaration: every `deliver_twitch_message` error code and its retryability.
#[derive(Debug, Clone, Serialize)]
pub struct TwitchDeliveryErrorDef {
    pub code: &'static str,
    pub retryable: bool,
}

pub const TWITCH_DELIVERY_ERRORS: &[TwitchDeliveryErrorDef] = &[
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::EMPTY_TEXT,
        retryable: false,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::UNAVAILABLE,
        retryable: true,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::SEND_FAILED,
        retryable: true,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::DELIVERY_UNKNOWN,
        retryable: false,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::QUEUE_FULL,
        retryable: true,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::TOO_LONG,
        retryable: false,
    },
    TwitchDeliveryErrorDef {
        code: twitch_delivery::error_code::PARTIAL_DELIVERY,
        retryable: false,
    },
];

pub fn twitch_delivery_error_code_to_retryable(code: &str) -> bool {
    TWITCH_DELIVERY_ERRORS
        .iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("unknown deliver_twitch_message error code: {code}"))
        .retryable
}

/// Stable payload for the `twitch-delivery-failed` event. Carries only the
/// canonical code and its retryability (derived from the exhaustive table) —
/// never message text, channel, username or token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TwitchDeliveryFailure {
    pub code: &'static str,
    pub retryable: bool,
}

impl TwitchDeliveryFailure {
    pub fn new(code: &'static str) -> Self {
        Self {
            code,
            retryable: twitch_delivery_error_code_to_retryable(code),
        }
    }
}

/// Stable error envelope serialized by Tauri command rejections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
#[derive(Serialize)]
struct SpeechErrorFixture {
    codes: &'static [SpeechErrorDef],
    envelope: CommandError,
}

#[cfg(test)]
mod tests {
    use super::{
        speech::error_code, twitch_delivery, CommandError, SpeechErrorFixture,
        SPEECH_SUBMIT_ERRORS, TWITCH_DELIVERY_ERRORS,
    };

    fn speech_error_fixture() -> SpeechErrorFixture {
        SpeechErrorFixture {
            codes: SPEECH_SUBMIT_ERRORS,
            envelope: CommandError::new(
                error_code::QUEUE_FULL,
                "queue full",
                super::speech_error_code_to_retryable(error_code::QUEUE_FULL),
            ),
        }
    }

    #[test]
    fn command_error_has_stable_wire_shape() {
        let error = CommandError::new("speech.queue_full", "queue full", true);

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "speech.queue_full",
                "message": "queue full",
                "retryable": true
            })
        );
    }

    #[test]
    fn speech_contract_error_defs_are_exhaustive_and_unique() {
        let codes: Vec<&str> = SPEECH_SUBMIT_ERRORS.iter().map(|d| d.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes.len(), sorted.len(), "duplicate error codes");
        assert_eq!(
            codes.len(),
            5,
            "expected exactly 5 submit_speech error codes"
        );
    }

    #[test]
    fn speech_contract_error_fixture_is_current() {
        let fixture = speech_error_fixture();
        let current = serde_json::to_value(&fixture).expect("serialize");
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/speech/speech-errors.json"
        );
        let on_disk_raw = std::fs::read_to_string(fixture_path).unwrap_or_else(|_| {
            panic!(
                "Fixture 'speech-errors.json' missing — regenerate with \
                 `cargo test speech_contract_fixtures_regenerate -- --ignored`"
            )
        });
        let on_disk =
            serde_json::from_str::<serde_json::Value>(&on_disk_raw).unwrap_or_else(|err| {
                panic!(
                    "Fixture 'speech-errors.json' is not valid JSON: {err} — regenerate with \
                 `cargo test speech_contract_fixtures_regenerate -- --ignored`"
                )
            });
        assert_eq!(
            current, on_disk,
            "Fixture 'speech-errors.json' is stale — regenerate with \
             `cargo test speech_contract_fixtures_regenerate -- --ignored`"
        );
    }

    #[test]
    fn speech_contract_retryability_is_consistent_with_constants() {
        let def = |code: &str| -> &super::SpeechErrorDef {
            SPEECH_SUBMIT_ERRORS
                .iter()
                .find(|d| d.code == code)
                .unwrap()
        };

        assert!(!def(error_code::SNAPSHOT_UNAVAILABLE).retryable);
        assert!(!def(error_code::EMPTY_TEXT).retryable);
        assert!(def(error_code::QUEUE_FULL).retryable);
        assert!(!def(error_code::QUEUE_REJECTED).retryable);
        assert!(!def(error_code::TWITCH_ONLY_ROUTE).retryable);
    }

    #[test]
    fn speech_contract_lookup_finds_all_known_codes() {
        for def in SPEECH_SUBMIT_ERRORS {
            let found = super::speech_error_code_to_retryable(def.code);
            assert_eq!(found, def.retryable);
        }
    }

    #[test]
    #[should_panic(expected = "unknown submit_speech error code")]
    fn speech_contract_lookup_rejects_unknown_code() {
        super::speech_error_code_to_retryable("nonexistent.code");
    }

    #[test]
    fn twitch_delivery_error_defs_are_exhaustive_and_unique() {
        let codes: Vec<&str> = TWITCH_DELIVERY_ERRORS.iter().map(|d| d.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes.len(), sorted.len(), "duplicate error codes");
        assert_eq!(
            codes.len(),
            7,
            "expected exactly 7 deliver_twitch_message error codes"
        );
    }

    #[test]
    fn twitch_delivery_retryability_is_consistent_with_constants() {
        let def = |code: &str| -> &super::TwitchDeliveryErrorDef {
            TWITCH_DELIVERY_ERRORS
                .iter()
                .find(|d| d.code == code)
                .unwrap()
        };

        assert!(!def(twitch_delivery::error_code::EMPTY_TEXT).retryable);
        assert!(def(twitch_delivery::error_code::UNAVAILABLE).retryable);
        assert!(def(twitch_delivery::error_code::SEND_FAILED).retryable);
        assert!(!def(twitch_delivery::error_code::DELIVERY_UNKNOWN).retryable);
        assert!(def(twitch_delivery::error_code::QUEUE_FULL).retryable);
        assert!(!def(twitch_delivery::error_code::TOO_LONG).retryable);
        assert!(!def(twitch_delivery::error_code::PARTIAL_DELIVERY).retryable);
    }

    #[test]
    fn twitch_delivery_lookup_finds_all_known_codes() {
        for def in TWITCH_DELIVERY_ERRORS {
            let found = super::twitch_delivery_error_code_to_retryable(def.code);
            assert_eq!(found, def.retryable);
        }
    }

    #[test]
    #[should_panic(expected = "unknown deliver_twitch_message error code")]
    fn twitch_delivery_lookup_rejects_unknown_code() {
        super::twitch_delivery_error_code_to_retryable("nonexistent.code");
    }

    #[test]
    fn twitch_delivery_failure_payload_derives_retryability_from_canonical_table() {
        for def in TWITCH_DELIVERY_ERRORS {
            let payload = super::TwitchDeliveryFailure::new(def.code);
            assert_eq!(payload.code, def.code);
            assert_eq!(payload.retryable, def.retryable);
        }
    }

    #[test]
    fn twitch_delivery_failure_payload_has_stable_wire_shape() {
        let payload = super::TwitchDeliveryFailure::new(twitch_delivery::error_code::QUEUE_FULL);
        assert_eq!(
            serde_json::to_value(payload).unwrap(),
            serde_json::json!({
                "code": "twitch.queue_full",
                "retryable": true
            })
        );
    }

    /// Regenerates all speech contract fixture files. Excluded from standard test runs.
    #[test]
    #[ignore]
    fn speech_contract_fixtures_regenerate() {
        let fixture_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/speech"
        );

        std::fs::create_dir_all(fixture_dir).expect("create fixture dir");

        let errors_fixture = speech_error_fixture();
        let errors_json =
            serde_json::to_string_pretty(&errors_fixture).expect("serialize speech errors");
        let errors_path = format!("{}/speech-errors.json", fixture_dir);
        std::fs::write(&errors_path, &errors_json).expect("write speech-errors fixture");
        eprintln!("Fixture written: {}", errors_path);
    }
}
