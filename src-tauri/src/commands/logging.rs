//! Logging commands for managing application logging settings

use crate::config::{LoggingSettings, SettingsManager};
use std::collections::HashMap;
use tauri::{AppHandle, State};

const VALID_LOG_LEVELS: &[&str] = &["error", "warn", "info", "debug", "trace"];

/// Quiet dependency defaults, followed by explicit RUST_LOG directives.
/// settings.logging.module_levels is applied by the caller last.
pub(crate) fn build_env_filter(
    default_level: tracing::Level,
    rust_log: Option<&str>,
) -> tracing_subscriber::EnvFilter {
    let mut directives = default_level.to_string();
    if matches!(
        default_level,
        tracing::Level::INFO | tracing::Level::DEBUG | tracing::Level::TRACE
    ) {
        // ONNX Runtime and libsql (Telegram session storage) are noisy at debug/trace.
        directives.push_str(",ort=warn,libsql=warn");
    }
    if let Some(explicit) = rust_log {
        directives.push(',');
        directives.push_str(explicit);
    }
    tracing_subscriber::EnvFilter::builder().parse_lossy(directives)
}

/// Validate log level string
fn validate_log_level(level: &str) -> Result<(), String> {
    if VALID_LOG_LEVELS.contains(&level) {
        Ok(())
    } else {
        Err(format!(
            "Invalid log level '{}'. Valid values: {}",
            level,
            VALID_LOG_LEVELS.join(", ")
        ))
    }
}

/// Validate module log levels
pub fn validate_module_levels(levels: &HashMap<String, String>) -> Result<(), String> {
    for level in levels.values() {
        // Validate log level value
        validate_log_level(level)?;

        // Note: Module name format validation is intentionally lenient.
        // tracing crate accepts various module formats, including:
        // - "crate::module::submodule" (standard Rust path)
        // - "ttsbard" (top-level crate name)
        // - "crate" (bare crate name for dependencies)
        // This allows maximum flexibility for per-module log filtering.
    }
    Ok(())
}

/// Get logging settings
#[tauri::command]
pub fn get_logging_settings(
    settings_manager: State<'_, SettingsManager>,
) -> Result<LoggingSettings, String> {
    Ok(settings_manager.get_logging_settings())
}

/// Save logging settings
#[tauri::command]
pub async fn save_logging_settings(
    enabled: bool,
    level: String,
    app_handle: AppHandle,
    settings_manager: State<'_, SettingsManager>,
) -> Result<(), String> {
    // Validate log level before updating (no clone needed - pass as &str)
    validate_log_level(&level)?;

    // Atomically update logging settings
    super::persist_blocking(settings_manager.inner(), move |mgr| {
        mgr.update_logging(move |logging| {
            logging.enabled = enabled;
            logging.level = level;
        })
    })
    .await?;

    super::emit_settings_changed(&app_handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::{layer::Context, prelude::*, Layer};

    type RecordedEvent = (&'static str, tracing::Level);

    #[derive(Clone, Default)]
    struct Recorded(Arc<Mutex<Vec<RecordedEvent>>>);

    impl<S: Subscriber> Layer<S> for Recorded {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            let metadata = event.metadata();
            self.0
                .lock()
                .unwrap()
                .push((metadata.target(), *metadata.level()));
        }
    }

    fn record(filter: tracing_subscriber::EnvFilter) -> Vec<(&'static str, tracing::Level)> {
        let events = Recorded::default();
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(events.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "ort", "ort debug");
            tracing::warn!(target: "ort", "ort warning");
            tracing::error!(target: "ort", "ort error");
            tracing::debug!(target: "libsql", "session debug");
            tracing::warn!(target: "libsql", "session warning");
            tracing::error!(target: "libsql", "session error");
            tracing::info!(target: "audio_diag", "audio event");
        });
        let recorded = events.0.lock().unwrap().clone();
        recorded
    }

    #[test]
    fn quiet_defaults_do_not_expand_error_level() {
        let events = record(build_env_filter(tracing::Level::ERROR, None));
        assert_eq!(
            events,
            vec![
                ("ort", tracing::Level::ERROR),
                ("libsql", tracing::Level::ERROR)
            ]
        );
    }

    #[test]
    fn verbose_defaults_quiet_dependencies_but_keep_audio_events() {
        for level in [
            tracing::Level::INFO,
            tracing::Level::DEBUG,
            tracing::Level::TRACE,
        ] {
            let events = record(build_env_filter(level, None));
            assert_eq!(events.len(), 5);
            assert!(!events
                .iter()
                .any(|(_, level)| *level == tracing::Level::DEBUG));
            assert!(events.contains(&("audio_diag", tracing::Level::INFO)));
        }
    }

    #[test]
    fn explicit_environment_and_settings_override_quiet_defaults() {
        let filter = build_env_filter(tracing::Level::INFO, Some("ort=debug,libsql=debug"));
        assert_eq!(record(filter).len(), 7);
        let filter = build_env_filter(tracing::Level::INFO, Some("ort=debug,libsql=debug"))
            .add_directive("ort=error".parse().unwrap());
        let events = record(filter);
        assert_eq!(
            events.iter().filter(|(target, _)| *target == "ort").count(),
            1
        );
        assert!(events.contains(&("libsql", tracing::Level::DEBUG)));
    }
}
