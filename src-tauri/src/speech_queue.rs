use crate::config::{
    AiSettings, AudioEffectsSettings, AudioSettings, DspSettings, NetworkSettings,
};
use crate::preprocessor::TextPreprocessor;
use crate::stress::runtime::RuAccentRuntimeSlot;
use crate::tts::TtsProvider;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_ACTIVE_CAPACITY: usize = 50;

/// Terminal jobs (`Completed`/`Cancelled`) kept for replay and restore.
///
/// Non-terminal jobs are bounded separately by [`MAX_ACTIVE_CAPACITY`], so the
/// queue holds at most `MAX_ACTIVE_CAPACITY + MAX_TERMINAL_RETENTION` jobs and
/// `state()` stays bounded for long sessions.
pub const MAX_TERMINAL_RETENTION: usize = 50;

// ── AcceptedJob: returned by submit ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedJob {
    pub job_id: Uuid,
}

// ── Submission source metadata ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionSource {
    #[default]
    Editor,
    Server,
    Ocr,
}

// ── Delivery policy ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryPolicy {
    Editor {
        skip_twitch: bool,
        skip_webview: bool,
    },
    /// Source-neutral incoming delivery: audio is inherent and the flags
    /// control WebView/Twitch. Unlike [`DeliveryPolicy::Editor`], the text is
    /// never parsed for route prefixes — the complete submitted string is the
    /// content.
    Incoming {
        skip_twitch: bool,
        skip_webview: bool,
    },
    AudioOnly,
}

impl DeliveryPolicy {
    pub fn editor(skip_twitch: bool, skip_webview: bool) -> Self {
        DeliveryPolicy::Editor {
            skip_twitch,
            skip_webview,
        }
    }

    pub fn incoming(skip_twitch: bool, skip_webview: bool) -> Self {
        DeliveryPolicy::Incoming {
            skip_twitch,
            skip_webview,
        }
    }

    pub fn audio_only() -> Self {
        DeliveryPolicy::AudioOnly
    }

    pub fn skip_twitch(&self) -> bool {
        match self {
            DeliveryPolicy::Editor { skip_twitch, .. } => *skip_twitch,
            DeliveryPolicy::Incoming { skip_twitch, .. } => *skip_twitch,
            DeliveryPolicy::AudioOnly => true,
        }
    }

    pub fn skip_webview(&self) -> bool {
        match self {
            DeliveryPolicy::Editor { skip_webview, .. } => *skip_webview,
            DeliveryPolicy::Incoming { skip_webview, .. } => *skip_webview,
            DeliveryPolicy::AudioOnly => true,
        }
    }

    pub fn skip_vrchat(&self) -> bool {
        self.skip_webview()
    }
}

// ── WorkItem: returned by claim_next_generation ──

#[derive(Clone)]
pub(crate) struct WorkItem {
    pub job_id: Uuid,
    pub original_text: String,
    pub snapshot: Snapshot,
}

impl std::fmt::Debug for WorkItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkItem")
            .field("job_id", &self.job_id)
            .field("original_text", &self.original_text)
            .field("snapshot", &"<opaque>")
            .finish()
    }
}

// ── Snapshot: settings frozen at submit time ──

#[derive(Clone)]
pub struct Snapshot {
    pub provider: String,
    pub voice: String,
    pub source: SubmissionSource,
    pub delivery: DeliveryPolicy,
    pub ai_enabled: bool,
    pub number_conversion_enabled: bool,
    pub audio_effects: AudioEffectsSettings,
    pub dsp: DspSettings,
    pub audio: AudioSettings,
    pub ai: AiSettings,
    pub tts_provider: TtsProvider,
    pub preprocessor: Option<TextPreprocessor>,
    pub network_settings: NetworkSettings,
    pub accentor_runtime: Option<RuAccentRuntimeSlot>,
}

// ── Job status ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Generating,
    Ready,
    Playing,
    Completed,
    Failed,
    Cancelled,
}

// ── SpeechJob ──

#[derive(Clone)]
pub struct SpeechJob {
    pub job_id: Uuid,
    pub original_text: String,
    pub spoken_text: Option<String>,
    pub status: JobStatus,
    pub error: Option<String>,
    pub attempt: u32,
    pub created_at_ms: i64,
    pub last_activity_at_ms: i64,
    pub source: SubmissionSource,
    pub snapshot: Snapshot,
    pub handoff_guard: Arc<parking_lot::Mutex<()>>,
}

impl SpeechJob {
    fn new(original_text: String, snapshot: Snapshot) -> Self {
        let now_ms = Utc::now().timestamp_millis();
        let source = snapshot.source;
        Self {
            job_id: Uuid::new_v4(),
            original_text,
            spoken_text: None,
            status: JobStatus::Queued,
            error: None,
            attempt: 1,
            created_at_ms: now_ms,
            last_activity_at_ms: now_ms,
            source,
            snapshot,
            handoff_guard: Arc::new(parking_lot::Mutex::new(())),
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_job(
        job_id: Uuid,
        original_text: String,
        spoken_text: Option<String>,
        status: JobStatus,
        error: Option<String>,
        attempt: u32,
        created_at_ms: i64,
        last_activity_at_ms: i64,
        snapshot: Snapshot,
    ) -> Self {
        let source = snapshot.source;
        Self {
            job_id,
            original_text,
            spoken_text,
            status,
            error,
            attempt,
            created_at_ms,
            last_activity_at_ms,
            source,
            snapshot,
            handoff_guard: Arc::new(parking_lot::Mutex::new(())),
        }
    }
}

// ── Errors ──

#[derive(Debug, Clone, Error)]
pub enum QueueError {
    #[error("empty or whitespace-only text")]
    EmptyText,
    #[error("queue full: maximum {0} active jobs")]
    QueueFull(usize),
    #[error("job not found: {0}")]
    JobNotFound(Uuid),
    #[error("invalid transition: cannot move from {from:?} to {to}")]
    InvalidTransition { from: JobStatus, to: &'static str },
    #[error("job {0} is not the next actionable job")]
    NotNextActionable(Uuid),
}

// ── DTOs ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDto {
    pub job_id: Uuid,
    pub original_text: String,
    pub spoken_text: Option<String>,
    pub status: JobStatus,
    pub error: Option<String>,
    pub attempt: u32,
    pub created_at_ms: i64,
    pub last_activity_at_ms: i64,
    pub source: SubmissionSource,
}

impl From<&SpeechJob> for JobDto {
    fn from(job: &SpeechJob) -> Self {
        Self {
            job_id: job.job_id,
            original_text: job.original_text.clone(),
            spoken_text: job.spoken_text.clone(),
            status: job.status,
            error: job.error.clone(),
            attempt: job.attempt,
            created_at_ms: job.created_at_ms,
            last_activity_at_ms: job.last_activity_at_ms,
            source: job.source,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechQueueStateDto {
    pub jobs: Vec<JobDto>,
}

// ── SpeechQueue ──

pub struct SpeechQueue {
    jobs: VecDeque<SpeechJob>,
}

impl Default for SpeechQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechQueue {
    pub fn new() -> Self {
        Self {
            jobs: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn push_job(&mut self, job: SpeechJob) {
        self.jobs.push_back(job);
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn submit(&mut self, text: &str, snapshot: Snapshot) -> Result<Uuid, QueueError> {
        if text.trim().is_empty() {
            return Err(QueueError::EmptyText);
        }
        if self.active_count() >= MAX_ACTIVE_CAPACITY {
            return Err(QueueError::QueueFull(MAX_ACTIVE_CAPACITY));
        }
        let job = SpeechJob::new(text.to_string(), snapshot);
        let id = job.job_id;
        self.jobs.push_back(job);
        Ok(id)
    }

    pub fn active_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.status != JobStatus::Completed && j.status != JobStatus::Cancelled)
            .count()
    }

    pub fn state(&self) -> SpeechQueueStateDto {
        SpeechQueueStateDto {
            jobs: self.jobs.iter().map(JobDto::from).collect(),
        }
    }

    /// Drop the oldest retained terminal jobs.
    ///
    /// Called after every transition into a terminal state. Active jobs
    /// (`Queued`/`Generating`/`Ready`/`Playing`) and `Failed` jobs are never
    /// evicted: a failed job still requires an explicit retry/skip and is
    /// already bounded by [`MAX_ACTIVE_CAPACITY`].
    fn enforce_terminal_retention(&mut self) {
        let mut terminal = self
            .jobs
            .iter()
            .filter(|job| is_terminal(job.status))
            .count();
        while terminal > MAX_TERMINAL_RETENTION {
            let Some(index) = self.jobs.iter().position(|job| is_terminal(job.status)) else {
                break;
            };
            self.jobs.remove(index);
            terminal -= 1;
        }
    }

    pub fn next_actionable(&self) -> Option<Uuid> {
        for job in &self.jobs {
            match job.status {
                JobStatus::Generating => return None,
                JobStatus::Queued => return Some(job.job_id),
                JobStatus::Failed
                | JobStatus::Completed
                | JobStatus::Cancelled
                | JobStatus::Ready
                | JobStatus::Playing => continue,
            }
        }
        None
    }

    fn get_mut(&mut self, job_id: Uuid) -> Result<&mut SpeechJob, QueueError> {
        self.jobs
            .iter_mut()
            .find(|j| j.job_id == job_id)
            .ok_or(QueueError::JobNotFound(job_id))
    }

    pub fn start_generation(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        if !self.jobs.iter().any(|j| j.job_id == job_id) {
            return Err(QueueError::JobNotFound(job_id));
        }
        let actionable = self.next_actionable();
        if actionable != Some(job_id) {
            return Err(QueueError::NotNextActionable(job_id));
        }
        let job = self.get_mut(job_id)?;
        job.status = JobStatus::Generating;
        Ok(())
    }

    pub(crate) fn claim_next_generation(&mut self) -> Result<Option<WorkItem>, QueueError> {
        let job_id = match self.next_actionable() {
            Some(id) => id,
            None => return Ok(None),
        };
        let job = self.get_mut(job_id)?;
        job.status = JobStatus::Generating;
        Ok(Some(WorkItem {
            job_id: job.job_id,
            original_text: job.original_text.clone(),
            snapshot: job.snapshot.clone(),
        }))
    }

    pub fn fail_playback(&mut self, job_id: Uuid, error_msg: String) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        match job.status {
            JobStatus::Ready | JobStatus::Playing => {
                job.status = JobStatus::Failed;
                job.error = Some(error_msg);
                Ok(())
            }
            JobStatus::Cancelled if job.error.is_none() && job.spoken_text.is_some() => {
                job.status = JobStatus::Failed;
                job.error = Some(error_msg);
                Ok(())
            }
            _ => Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Failed",
            }),
        }
    }

    pub fn mark_ready(&mut self, job_id: Uuid, spoken_text: String) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Generating {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Ready",
            });
        }
        job.status = JobStatus::Ready;
        job.spoken_text = Some(spoken_text);
        Ok(())
    }

    pub fn set_spoken_text(&mut self, job_id: Uuid, text: String) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Generating {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "spoken text",
            });
        }
        job.spoken_text = Some(text);
        Ok(())
    }

    pub fn fail_generation(&mut self, job_id: Uuid, error_msg: String) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Generating {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Failed",
            });
        }
        job.status = JobStatus::Failed;
        job.error = Some(error_msg);
        Ok(())
    }

    pub fn mark_playing(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        match job.status {
            JobStatus::Ready => {
                job.status = JobStatus::Playing;
                Ok(())
            }
            JobStatus::Cancelled if job.error.is_none() && job.spoken_text.is_some() => {
                job.status = JobStatus::Playing;
                Ok(())
            }
            _ => Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Playing",
            }),
        }
    }

    pub fn mark_completed(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Playing {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Completed",
            });
        }
        job.status = JobStatus::Completed;
        self.enforce_terminal_retention();
        Ok(())
    }

    pub fn retry_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Failed {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Queued (retry)",
            });
        }
        job.status = JobStatus::Queued;
        job.error = None;
        job.spoken_text = None;
        job.attempt += 1;
        bump_activity(job);
        Ok(())
    }

    pub fn cancel_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        match job.status {
            JobStatus::Queued | JobStatus::Failed => {
                job.status = JobStatus::Cancelled;
            }
            _ => {
                return Err(QueueError::InvalidTransition {
                    from: job.status,
                    to: "Cancelled",
                })
            }
        }
        self.enforce_terminal_retention();
        Ok(())
    }

    pub fn cancel_ready_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Ready {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Cancelled",
            });
        }
        job.status = JobStatus::Cancelled;
        self.enforce_terminal_retention();
        Ok(())
    }

    pub fn cancel_generating_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Generating {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Cancelled",
            });
        }
        job.status = JobStatus::Cancelled;
        job.error = None;
        self.enforce_terminal_retention();
        Ok(())
    }

    pub fn get_handoff_guard(&self, job_id: Uuid) -> Option<Arc<parking_lot::Mutex<()>>> {
        self.jobs
            .iter()
            .find(|j| j.job_id == job_id)
            .map(|j| j.handoff_guard.clone())
    }

    pub fn skip_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let job = self.get_mut(job_id)?;
        if job.status != JobStatus::Failed {
            return Err(QueueError::InvalidTransition {
                from: job.status,
                to: "Skipped (Cancelled)",
            });
        }
        job.status = JobStatus::Cancelled;
        bump_activity(job);
        self.enforce_terminal_retention();
        Ok(())
    }

    pub fn restore_cancelled_job(&mut self, job_id: Uuid) -> Result<(), QueueError> {
        let (status, error_is_some) = match self.jobs.iter().find(|j| j.job_id == job_id) {
            Some(job) => (job.status, job.error.is_some()),
            None => return Err(QueueError::JobNotFound(job_id)),
        };
        if status != JobStatus::Cancelled {
            return Err(QueueError::InvalidTransition {
                from: status,
                to: "Queued (restore)",
            });
        }
        if error_is_some {
            return Err(QueueError::InvalidTransition {
                from: status,
                to: "Queued (restore)",
            });
        }
        if self.active_count() >= MAX_ACTIVE_CAPACITY {
            return Err(QueueError::QueueFull(MAX_ACTIVE_CAPACITY));
        }
        let job = self.get_mut(job_id)?;
        job.status = JobStatus::Queued;
        job.spoken_text = None;
        job.handoff_guard = Arc::new(parking_lot::Mutex::new(()));
        bump_activity(job);
        Ok(())
    }

    pub fn has_job(&self, job_id: Uuid) -> bool {
        self.jobs.iter().any(|j| j.job_id == job_id)
    }

    pub fn get_status(&self, job_id: Uuid) -> Option<JobStatus> {
        self.jobs
            .iter()
            .find(|j| j.job_id == job_id)
            .map(|j| j.status)
    }

    pub fn get_spoken_text(&self, job_id: Uuid) -> Option<String> {
        self.jobs
            .iter()
            .find(|j| j.job_id == job_id)
            .and_then(|j| j.spoken_text.clone())
    }

    pub fn get_error(&self, job_id: Uuid) -> Option<String> {
        self.jobs
            .iter()
            .find(|j| j.job_id == job_id)
            .and_then(|j| j.error.clone())
    }

    pub fn touch_activity(&mut self, job_id: Uuid) -> bool {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            let now = Utc::now().timestamp_millis();
            if now > job.last_activity_at_ms {
                job.last_activity_at_ms = now;
            } else {
                job.last_activity_at_ms += 1;
            }
            true
        } else {
            false
        }
    }
}

fn bump_activity(job: &mut SpeechJob) {
    let now = Utc::now().timestamp_millis();
    if now > job.last_activity_at_ms {
        job.last_activity_at_ms = now;
    } else {
        job.last_activity_at_ms += 1;
    }
}

fn is_terminal(status: JobStatus) -> bool {
    matches!(status, JobStatus::Completed | JobStatus::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AiOpenAiSettings;

    fn snap() -> Snapshot {
        Snapshot {
            provider: "test-provider".into(),
            voice: "test-voice".into(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(false, false),
            ai_enabled: false,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings::default(),
            tts_provider: TtsProvider::Local(
                crate::tts::local_http_server::LocalHttpServerTts::new(),
            ),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        }
    }

    fn text_err(e: &QueueError) -> String {
        e.to_string()
    }

    // ── submission source / delivery policy ──

    #[test]
    fn submission_source_serializes_to_editor_server_and_ocr() {
        assert_eq!(
            serde_json::to_string(&SubmissionSource::Editor).unwrap(),
            "\"editor\""
        );
        assert_eq!(
            serde_json::to_string(&SubmissionSource::Server).unwrap(),
            "\"server\""
        );
        assert_eq!(
            serde_json::to_string(&SubmissionSource::Ocr).unwrap(),
            "\"ocr\""
        );
    }

    #[test]
    fn source_metadata_reaches_job_dto() {
        let mut editor_q = SpeechQueue::new();
        editor_q.submit("editor text", snap()).unwrap();
        assert_eq!(editor_q.state().jobs[0].source, SubmissionSource::Editor);

        let mut server_snapshot = snap();
        server_snapshot.source = SubmissionSource::Server;
        server_snapshot.delivery = DeliveryPolicy::AudioOnly;

        let mut server_q = SpeechQueue::new();
        server_q.submit("server text", server_snapshot).unwrap();
        assert_eq!(server_q.state().jobs[0].source, SubmissionSource::Server);

        let mut ocr_snapshot = snap();
        ocr_snapshot.source = SubmissionSource::Ocr;
        ocr_snapshot.delivery = DeliveryPolicy::AudioOnly;

        let mut ocr_q = SpeechQueue::new();
        ocr_q.submit("ocr text", ocr_snapshot).unwrap();
        assert_eq!(ocr_q.state().jobs[0].source, SubmissionSource::Ocr);
    }

    #[test]
    fn editor_delivery_reflects_prefix_skip_flags() {
        let both = DeliveryPolicy::editor(true, true);
        assert!(both.skip_twitch());
        assert!(both.skip_webview());
        assert!(both.skip_vrchat());

        let twitch_only = DeliveryPolicy::editor(true, false);
        assert!(twitch_only.skip_twitch());
        assert!(!twitch_only.skip_webview());
        assert!(!twitch_only.skip_vrchat());

        let everywhere = DeliveryPolicy::editor(false, false);
        assert!(!everywhere.skip_twitch());
        assert!(!everywhere.skip_webview());
        assert!(!everywhere.skip_vrchat());
    }

    #[test]
    fn audio_only_delivery_forces_skip_webview_and_twitch() {
        let policy = DeliveryPolicy::AudioOnly;
        assert!(policy.skip_twitch());
        assert!(policy.skip_webview());
        assert!(policy.skip_vrchat());
    }

    #[test]
    fn incoming_delivery_skip_vrchat_matrix() {
        // Everywhere: both Twitch & WebView enabled -> delivers to VRChat
        let everywhere = DeliveryPolicy::incoming(false, false);
        assert!(!everywhere.skip_vrchat());

        // AudioWebview (no_twitch): Twitch skipped, WebView enabled -> delivers to VRChat
        let no_twitch = DeliveryPolicy::incoming(true, false);
        assert!(!no_twitch.skip_vrchat());

        // AudioTwitch (twitch_only): Twitch enabled, WebView skipped -> skips VRChat
        let twitch_only = DeliveryPolicy::incoming(false, true);
        assert!(twitch_only.skip_vrchat());

        // AudioOnly (voice_only): both Twitch & WebView skipped -> skips VRChat
        let voice_only = DeliveryPolicy::incoming(true, true);
        assert!(voice_only.skip_vrchat());
    }

    // ── submit ──

    #[test]
    fn submit_creates_job_with_uuid() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello world", snap()).unwrap();
        let state = q.state();
        assert_eq!(state.jobs.len(), 1);
        assert_eq!(state.jobs[0].job_id, id);
        assert_eq!(state.jobs[0].original_text, "hello world");
        assert_eq!(state.jobs[0].status, JobStatus::Queued);
        assert_eq!(state.jobs[0].attempt, 1);
    }

    #[test]
    fn submit_empty_text_rejected() {
        let mut q = SpeechQueue::new();
        let err = q.submit("", snap()).unwrap_err();
        assert!(text_err(&err).contains("empty"));
        assert!(q.is_empty());
    }

    #[test]
    fn submit_whitespace_only_rejected() {
        let mut q = SpeechQueue::new();
        let err = q.submit("   \t\n   ", snap()).unwrap_err();
        assert!(text_err(&err).contains("empty"));
        assert!(q.is_empty());
    }

    #[test]
    fn submit_preserves_original_text_with_whitespace() {
        let mut q = SpeechQueue::new();
        let id = q.submit("  hello world  ", snap()).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].original_text, "  hello world  ");
        assert_eq!(state.jobs[0].job_id, id);
    }

    #[test]
    fn submit_preserves_leading_whitespace_text() {
        let mut q = SpeechQueue::new();
        let text = " !hello";
        let id = q.submit(text, snap()).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].original_text, " !hello");
        assert_eq!(state.jobs[0].job_id, id);
    }

    #[test]
    fn duplicate_texts_get_distinct_ids() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("hello", snap()).unwrap();
        let id2 = q.submit("hello", snap()).unwrap();
        assert_ne!(id1, id2);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn submission_fifo_order() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        let id3 = q.submit("third", snap()).unwrap();

        let state = q.state();
        assert_eq!(state.jobs.len(), 3);
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[1].job_id, id2);
        assert_eq!(state.jobs[2].job_id, id3);
    }

    // ── next_actionable ──

    #[test]
    fn next_actionable_first_queued() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let _id2 = q.submit("second", snap()).unwrap();
        assert_eq!(q.next_actionable(), Some(id1));
    }

    #[test]
    fn next_actionable_skips_terminal_states() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        let _id3 = q.submit("third", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.mark_ready(id1, "first".into()).unwrap();
        q.mark_playing(id1).unwrap();
        q.mark_completed(id1).unwrap();

        assert_eq!(q.next_actionable(), Some(id2));
    }

    #[test]
    fn next_actionable_skips_failed_and_selects_next_queued() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "provider error".into()).unwrap();

        assert_eq!(q.next_actionable(), Some(id2));
    }

    #[test]
    fn next_actionable_none_when_blocked_by_generating() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let _id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();

        assert_eq!(q.next_actionable(), None);
    }

    #[test]
    fn next_actionable_skips_ready_playing_completed_cancelled() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("job1", snap()).unwrap();
        q.start_generation(id1).unwrap();
        q.mark_ready(id1, "text".into()).unwrap();
        q.mark_playing(id1).unwrap();
        q.mark_completed(id1).unwrap();

        let id2 = q.submit("job2", snap()).unwrap();
        q.cancel_job(id2).unwrap();

        let id3 = q.submit("job3", snap()).unwrap();
        assert_eq!(q.next_actionable(), Some(id3));
    }

    // ── claim_next_generation ──

    #[test]
    fn claim_returns_first_queued_with_snapshot() {
        let mut q = SpeechQueue::new();
        let s = snap();
        q.submit("first", s.clone()).unwrap();
        q.submit("second", snap()).unwrap();

        let item = q.claim_next_generation().unwrap().expect("should claim");
        assert_eq!(item.original_text, "first");
        assert_eq!(item.job_id, q.state().jobs[0].job_id);
        assert_eq!(item.snapshot.voice, "test-voice");
    }

    #[test]
    fn claim_returns_none_when_idle() {
        let mut q = SpeechQueue::new();
        assert!(q.claim_next_generation().unwrap().is_none());
    }

    #[test]
    fn claim_prevents_duplicate_in_flight() {
        let mut q = SpeechQueue::new();
        q.submit("first", snap()).unwrap();

        q.claim_next_generation().unwrap().expect("first claim");
        assert!(q.claim_next_generation().unwrap().is_none());
    }

    #[test]
    fn claim_returns_next_queued_after_failed_head() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "err".into()).unwrap();

        let item = q
            .claim_next_generation()
            .unwrap()
            .expect("should claim second");
        assert_eq!(item.job_id, id2);
        assert_eq!(item.original_text, "second");
        assert_eq!(q.state().jobs[0].status, JobStatus::Failed);
    }

    #[test]
    fn claim_returns_none_when_blocked_by_generating() {
        let mut q = SpeechQueue::new();
        q.submit("first", snap()).unwrap();
        q.submit("second", snap()).unwrap();

        q.claim_next_generation().unwrap();
        assert!(q.claim_next_generation().unwrap().is_none());
    }

    #[test]
    fn claim_after_retry_of_failed_head() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "err".into()).unwrap();
        q.retry_job(id1).unwrap();

        let item = q.claim_next_generation().unwrap().expect("should claim");
        assert_eq!(item.original_text, "first");
    }

    #[test]
    fn claim_after_skip_of_failed_head_advances() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "err".into()).unwrap();
        q.skip_job(id1).unwrap();

        let item = q.claim_next_generation().unwrap().expect("should claim");
        assert_eq!(item.original_text, "second");
    }

    // ── fail_playback ──

    #[test]
    fn fail_playback_from_ready() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();

        q.fail_playback(id, "playback error".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("playback error"));
        assert_eq!(job.spoken_text.as_deref(), Some("spoken"));
        assert_eq!(job.attempt, 1);
    }

    #[test]
    fn fail_playback_from_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();
        q.mark_playing(id).unwrap();

        q.fail_playback(id, "device lost".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("device lost"));
        assert_eq!(job.spoken_text.as_deref(), Some("spoken"));
        assert_eq!(job.attempt, 1);
    }

    #[test]
    fn fail_playback_rejects_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn fail_playback_rejects_generating() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);
    }

    #[test]
    fn fail_playback_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn fail_playback_rejects_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn fail_playback_rejects_failed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "prev".into()).unwrap();
        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn fail_playback_unknown_id() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        let err = q.fail_playback(fake, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    #[test]
    fn fail_playback_preserves_spoken_text_and_attempt() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "preserve-me".into()).unwrap();
        q.fail_playback(id, "oops".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.spoken_text.as_deref(), Some("preserve-me"));
        assert_eq!(job.attempt, 1);
    }

    // ── start_generation ──

    #[test]
    fn start_generation_on_actionable_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Generating);
    }

    #[test]
    fn start_generation_rejects_non_next_job() {
        let mut q = SpeechQueue::new();
        let _id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        let err = q.start_generation(id2).unwrap_err();
        assert!(text_err(&err).contains("not the next actionable job"));
    }

    #[test]
    fn start_generation_accepts_next_queued_after_failed_head() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "error".into()).unwrap();
        q.start_generation(id2).unwrap();
        assert_eq!(q.state().jobs[1].status, JobStatus::Generating);
    }

    #[test]
    fn start_generation_rejects_unknown_id() {
        let mut q = SpeechQueue::new();
        let fake_id = Uuid::new_v4();
        let err = q.start_generation(fake_id).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    #[test]
    fn start_generation_rejects_when_prior_generating() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        q.start_generation(id1).unwrap();
        let err = q.start_generation(id2).unwrap_err();
        assert!(text_err(&err).contains("not the next actionable job"));
    }

    // ── mark_ready ──

    #[test]
    fn mark_ready_from_generating_records_spoken_text() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "hello world".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Ready);
        assert_eq!(job.spoken_text.as_deref(), Some("hello world"));
    }

    #[test]
    fn mark_ready_rejects_from_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.mark_ready(id, "text".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn retry_clears_spoken_text_set_before_failure() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.set_spoken_text(id, "partial".into()).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.retry_job(id).unwrap();
        assert_eq!(q.state().jobs[0].spoken_text, None);
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn retry_then_second_attempt_sets_new_spoken_text() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();

        q.start_generation(id).unwrap();
        q.set_spoken_text(id, "v1".into()).unwrap();
        q.fail_generation(id, "err".into()).unwrap();

        q.retry_job(id).unwrap();
        assert_eq!(q.state().jobs[0].spoken_text, None);
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);

        q.start_generation(id).unwrap();
        q.set_spoken_text(id, "v2".into()).unwrap();
        q.mark_ready(id, "v2".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Ready);
        assert_eq!(job.spoken_text.as_deref(), Some("v2"));
    }

    // ── fail_generation ──

    #[test]
    fn fail_generation_sets_failed_and_error() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "TTS timeout".into()).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("TTS timeout"));
    }

    #[test]
    fn fail_generation_preserves_failed_status_and_error() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "provider down".into()).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("provider down"));
        assert_eq!(q.next_actionable(), Some(id2));
    }

    #[test]
    fn fail_generation_preserves_idempotent_state_on_error() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.fail_generation(id, "oops".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
        assert_eq!(q.state().jobs[0].error, None);
    }

    // ── mark_playing / mark_completed ──

    #[test]
    fn full_happy_path() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();

        q.start_generation(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);

        q.mark_ready(id, "hello".into()).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Ready);

        q.mark_playing(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);

        q.mark_completed(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Completed);
    }

    #[test]
    fn mark_playing_rejects_from_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.mark_playing(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn mark_completed_rejects_from_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.mark_completed(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn mark_playing_rejects_from_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        let err = q.mark_playing(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn mark_completed_rejects_from_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.mark_completed(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn mark_playing_accepts_cancelled_ready_origin() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken form".into()).unwrap();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_some());
        assert!(q.state().jobs[0].error.is_none());

        q.mark_playing(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);
    }

    #[test]
    fn mark_playing_rejects_cancelled_queued_origin() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_none());

        let err = q.mark_playing(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn mark_playing_rejects_cancelled_failed_origin() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].error.is_some());

        let err = q.mark_playing(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn fail_playback_accepts_cancelled_ready_origin() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken form".into()).unwrap();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_some());
        assert!(q.state().jobs[0].error.is_none());

        q.fail_playback(id, "sink error".into()).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Failed);
        assert_eq!(q.state().jobs[0].error.as_deref(), Some("sink error"));
    }

    #[test]
    fn fail_playback_rejects_cancelled_queued_origin() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_none());

        let err = q.fail_playback(id, "err".into()).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    // ── retry ──

    #[test]
    fn retry_failed_job_resets_to_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "timeout".into()).unwrap();

        q.retry_job(id).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.error, None);
        assert_eq!(job.spoken_text, None);
        assert_eq!(job.attempt, 2);
    }

    #[test]
    fn retry_preserves_fifo_position() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "err".into()).unwrap();

        q.retry_job(id1).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[1].job_id, id2);
        assert_eq!(q.next_actionable(), Some(id1));
    }

    #[test]
    fn retry_increments_attempt_multiple_times() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err1".into()).unwrap();

        q.retry_job(id).unwrap();
        assert_eq!(q.state().jobs[0].attempt, 2);

        q.start_generation(id).unwrap();
        q.fail_generation(id, "err2".into()).unwrap();

        q.retry_job(id).unwrap();
        assert_eq!(q.state().jobs[0].attempt, 3);
    }

    #[test]
    fn retry_rejects_non_failed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.retry_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn retry_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.retry_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn retry_rejects_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        let err = q.retry_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    // ── cancel ──

    #[test]
    fn cancel_queued_job() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn cancel_failed_job_then_next_queued_selected() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "error".into()).unwrap();

        q.cancel_job(id1).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Cancelled);
        assert_eq!(q.next_actionable(), Some(id2));
    }

    #[test]
    fn cancel_later_job_keeps_prior_failed_and_next_queued() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let _id2 = q.submit("second", snap()).unwrap();
        let id3 = q.submit("third", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "error".into()).unwrap();

        q.cancel_job(id3).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[2].status, JobStatus::Cancelled);
        assert_eq!(state.jobs[1].status, JobStatus::Queued);
        assert_eq!(q.next_actionable(), Some(state.jobs[1].job_id));
    }

    #[test]
    fn cancel_rejects_generating() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let err = q.cancel_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);
    }

    #[test]
    fn cancel_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.cancel_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn cancel_ready_job_preserves_spoken_text() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken form".into()).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Ready);
        assert_eq!(q.state().jobs[0].spoken_text, Some("spoken form".into()));

        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert_eq!(q.state().jobs[0].spoken_text, Some("spoken form".into()));
    }

    #[test]
    fn cancel_ready_job_does_not_update_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        let before = q.state().jobs[0].last_activity_at_ms;

        q.cancel_ready_job(id).unwrap();
        let after = q.state().jobs[0].last_activity_at_ms;
        assert_eq!(after, before);
    }

    #[test]
    fn cancel_queued_job_does_not_update_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let before = q.state().jobs[0].last_activity_at_ms;

        q.cancel_job(id).unwrap();
        let after = q.state().jobs[0].last_activity_at_ms;
        assert_eq!(after, before);
    }

    #[test]
    fn cancel_ready_job_removes_from_active_count() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        assert_eq!(q.active_count(), 1);

        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.active_count(), 0);
    }

    #[test]
    fn cancel_rejects_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        let err = q.cancel_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);
    }

    #[test]
    fn cancel_ready_job_rejects_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn cancel_ready_job_rejects_generating() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);
    }

    #[test]
    fn cancel_ready_job_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn cancel_ready_job_rejects_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn cancel_ready_job_rejects_failed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "prev".into()).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Failed);
    }

    #[test]
    fn cancel_ready_job_rejects_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        let err = q.cancel_ready_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);
    }

    #[test]
    fn cancel_ready_job_unknown_id() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        let err = q.cancel_ready_job(fake).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    // ── skip ──

    #[test]
    fn skip_rejects_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn skip_failed_job_cancels_and_selects_next() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "err".into()).unwrap();

        q.skip_job(id1).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Cancelled);
        assert_eq!(q.next_actionable(), Some(id2));
    }

    #[test]
    fn skip_rejects_generating() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);
    }

    #[test]
    fn skip_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn skip_rejects_ready() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Ready);
    }

    #[test]
    fn skip_rejects_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);
    }

    #[test]
    fn skip_rejects_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        let err = q.skip_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn skip_unknown_id_errors() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        let err = q.skip_job(fake).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    // ── restore_cancelled_job ──

    #[test]
    fn restore_cancelled_queued_job_back_to_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);

        q.restore_cancelled_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn restore_preserves_job_id_text_snapshot_fifo_and_attempt() {
        let mut q = SpeechQueue::new();
        let id = q.submit("restore-me", snap()).unwrap();
        let _id2 = q.submit("second", snap()).unwrap();

        let snapshot_before = q.jobs[0].snapshot.provider.clone();
        let voice_before = q.jobs[0].snapshot.voice.clone();
        q.cancel_job(id).unwrap();

        q.restore_cancelled_job(id).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].job_id, id);
        assert_eq!(state.jobs[0].original_text, "restore-me");
        assert_eq!(state.jobs[0].status, JobStatus::Queued);
        assert_eq!(state.jobs[0].attempt, 1);
        assert_eq!(state.jobs[0].spoken_text, None);
        assert_eq!(state.jobs[0].error, None);
        // Snapshot fields preserved
        assert_eq!(q.jobs[0].snapshot.provider, snapshot_before);
        assert_eq!(q.jobs[0].snapshot.voice, voice_before);
        // FIFO position preserved
        assert_eq!(state.jobs[1].job_id, _id2);
    }

    #[test]
    fn restore_updates_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        let before_ms = q.state().jobs[0].last_activity_at_ms;

        q.restore_cancelled_job(id).unwrap();

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(after_ms > before_ms);
    }

    #[test]
    fn restore_rejects_non_cancelled_job() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.restore_cancelled_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn restore_rejects_failed_origin_cancelled_with_error() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.cancel_job(id).unwrap();
        // job is Cancelled with Some(error)
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].error.is_some());

        let err = q.restore_cancelled_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn restore_rejects_skipped_origin_cancelled_with_error() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.skip_job(id).unwrap();
        // job is Cancelled with Some(error)
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].error.is_some());

        let err = q.restore_cancelled_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    #[test]
    fn restore_rejects_unknown_id() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        let err = q.restore_cancelled_job(fake).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    #[test]
    fn restore_capacity_full_rejection_is_atomic() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            q.cancel_job(id).unwrap();
        }
        for i in 0..MAX_ACTIVE_CAPACITY {
            q.submit(&format!("active {}", i), snap()).unwrap();
        }
        // Now active_count == MAX_ACTIVE_CAPACITY
        assert_eq!(q.active_count(), MAX_ACTIVE_CAPACITY);

        let target_id = q.state().jobs[0].job_id;
        let state_before = serde_json::to_value(q.state()).unwrap();

        let err = q.restore_cancelled_job(target_id).unwrap_err();
        assert!(text_err(&err).contains("queue full"));

        let state_after = serde_json::to_value(q.state()).unwrap();
        assert_eq!(state_before, state_after);
    }

    #[test]
    fn restore_ready_cancelled_back_to_queued_clears_spoken_text() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken form".into()).unwrap();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_some());

        q.restore_cancelled_job(id).unwrap();
        let job = &q.state().jobs[0];
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.spoken_text, None);
        assert_eq!(job.error, None);
    }

    // ── capacity ──

    #[test]
    fn capacity_accepts_up_to_max() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            let text = format!("job {}", i);
            let id = q.submit(&text, snap()).unwrap();
            assert!(!id.is_nil());
        }
        assert_eq!(q.active_count(), MAX_ACTIVE_CAPACITY);
    }

    #[test]
    fn capacity_rejects_when_full() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            q.submit(&format!("job {}", i), snap()).unwrap();
        }
        let err = q.submit("overflow", snap()).unwrap_err();
        assert!(text_err(&err).contains("queue full"));
        assert_eq!(q.active_count(), MAX_ACTIVE_CAPACITY);
    }

    #[test]
    fn completed_jobs_do_not_consume_capacity() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            q.start_generation(id).unwrap();
            q.mark_ready(id, format!("text {}", i)).unwrap();
            q.mark_playing(id).unwrap();
            q.mark_completed(id).unwrap();
        }
        assert_eq!(q.active_count(), 0);
        let id = q.submit("new after completed", snap()).unwrap();
        assert!(!id.is_nil());
    }

    #[test]
    fn cancelled_jobs_do_not_consume_capacity() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            q.cancel_job(id).unwrap();
        }
        assert_eq!(q.active_count(), 0);
        let id = q.submit("new after cancelled", snap()).unwrap();
        assert!(!id.is_nil());
    }

    #[test]
    fn mixed_active_and_terminal_capacity() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            let id = q.submit(&format!("completed {}", i), snap()).unwrap();
            q.start_generation(id).unwrap();
            q.mark_ready(id, format!("text {}", i)).unwrap();
            q.mark_playing(id).unwrap();
            q.mark_completed(id).unwrap();
        }
        for i in 0..MAX_ACTIVE_CAPACITY {
            q.submit(&format!("queued {}", i), snap()).unwrap();
        }
        assert_eq!(q.active_count(), MAX_ACTIVE_CAPACITY);
        let err = q.submit("overflow", snap()).unwrap_err();
        assert!(text_err(&err).contains("queue full"));
    }

    #[test]
    fn capacity_queue_full_preserves_order() {
        let mut q = SpeechQueue::new();
        for i in 0..MAX_ACTIVE_CAPACITY {
            q.submit(&format!("job {}", i), snap()).unwrap();
        }
        let before = q.state();
        let _ = q.submit("should be rejected", snap());
        let after = q.state();
        assert_eq!(before.jobs.len(), after.jobs.len());
        for (a, b) in before.jobs.iter().zip(after.jobs.iter()) {
            assert_eq!(a.job_id, b.job_id);
            assert_eq!(a.status, b.status);
        }
    }

    // ── terminal retention ──

    fn complete_job(q: &mut SpeechQueue, id: Uuid) {
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".to_string()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
    }

    #[test]
    fn terminal_retention_bounds_completed_jobs() {
        let mut q = SpeechQueue::new();
        let mut ids = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 5) {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            complete_job(&mut q, id);
            ids.push(id);
        }

        assert_eq!(q.len(), MAX_TERMINAL_RETENTION);
        assert_eq!(q.state().jobs.len(), MAX_TERMINAL_RETENTION);
        assert!(q.len() <= MAX_ACTIVE_CAPACITY + MAX_TERMINAL_RETENTION);
        assert_eq!(q.active_count(), 0);

        for evicted in &ids[..5] {
            assert!(!q.has_job(*evicted), "oldest completed job must be evicted");
        }
        let retained: Vec<Uuid> = q.state().jobs.iter().map(|job| job.job_id).collect();
        assert_eq!(retained, ids[5..].to_vec());
    }

    #[test]
    fn terminal_retention_bounds_cancelled_jobs() {
        let mut q = SpeechQueue::new();
        let mut ids = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 5) {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            q.cancel_job(id).unwrap();
            ids.push(id);
        }

        assert_eq!(q.len(), MAX_TERMINAL_RETENTION);
        assert_eq!(q.state().jobs.len(), MAX_TERMINAL_RETENTION);
        assert_eq!(q.active_count(), 0);
        for evicted in &ids[..5] {
            assert!(!q.has_job(*evicted), "oldest cancelled job must be evicted");
        }
        assert!(q.has_job(ids[5]));
        assert!(q.has_job(*ids.last().unwrap()));
    }

    #[test]
    fn terminal_retention_evicts_oldest_terminal_across_kinds() {
        let mut q = SpeechQueue::new();
        let mut ids = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 1) {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            if i % 2 == 0 {
                complete_job(&mut q, id);
            } else {
                q.cancel_job(id).unwrap();
            }
            ids.push(id);
        }

        assert_eq!(q.len(), MAX_TERMINAL_RETENTION);
        assert!(!q.has_job(ids[0]), "oldest terminal job is evicted first");
        assert!(q.has_job(ids[1]));
        assert!(q.has_job(ids[MAX_TERMINAL_RETENTION]));
    }

    #[test]
    fn terminal_retention_never_evicts_active_jobs() {
        let mut q = SpeechQueue::new();
        let mut completed = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 5) {
            let id = q.submit(&format!("completed {}", i), snap()).unwrap();
            complete_job(&mut q, id);
            completed.push(id);
        }

        let mut queued = Vec::new();
        for i in 0..5 {
            queued.push(q.submit(&format!("queued {}", i), snap()).unwrap());
        }

        // The next terminal transition evicts the oldest terminal job only.
        complete_job(&mut q, queued[0]);

        for id in &queued[1..] {
            assert!(q.has_job(*id), "queued job must survive terminal eviction");
        }
        assert_eq!(q.get_status(queued[0]), Some(JobStatus::Completed));
        assert!(!q.has_job(completed[5]));
        assert!(q.has_job(completed[6]));
        assert_eq!(q.active_count(), 4);
        assert_eq!(q.len(), MAX_TERMINAL_RETENTION + 4);
    }

    #[test]
    fn terminal_retention_never_evicts_failed_jobs() {
        let mut q = SpeechQueue::new();
        let mut failed = Vec::new();
        for i in 0..45 {
            let id = q.submit(&format!("failed {}", i), snap()).unwrap();
            q.start_generation(id).unwrap();
            q.fail_generation(id, "boom".to_string()).unwrap();
            failed.push(id);
        }

        let mut completed = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 5) {
            let id = q.submit(&format!("completed {}", i), snap()).unwrap();
            complete_job(&mut q, id);
            completed.push(id);
        }

        for id in &failed {
            assert!(q.has_job(*id), "failed job must stay actionable");
        }
        assert!(!q.has_job(completed[4]));
        assert!(q.has_job(completed[5]));
        assert_eq!(q.active_count(), 45);
        assert_eq!(q.len(), 45 + MAX_TERMINAL_RETENTION);
    }

    #[test]
    fn terminal_retention_keeps_recent_job_restorable() {
        let mut q = SpeechQueue::new();
        let mut ids = Vec::new();
        for i in 0..(MAX_TERMINAL_RETENTION + 1) {
            let id = q.submit(&format!("job {}", i), snap()).unwrap();
            q.cancel_job(id).unwrap();
            ids.push(id);
        }

        let evicted = ids[0];
        let retained = ids[1];
        assert!(!q.has_job(evicted));
        assert_eq!(q.get_status(evicted), None);
        assert_eq!(q.get_spoken_text(evicted), None);
        assert!(matches!(
            q.restore_cancelled_job(evicted),
            Err(QueueError::JobNotFound(_))
        ));

        assert!(q.has_job(retained));
        q.restore_cancelled_job(retained).unwrap();
        assert_eq!(q.get_status(retained), Some(JobStatus::Queued));
        assert_eq!(q.active_count(), 1);
    }

    // ── unknown id ──

    #[test]
    fn unknown_id_errors_on_all_operations() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();

        for op_name in &[
            "start_generation",
            "set_spoken_text",
            "mark_ready",
            "fail_generation",
            "mark_playing",
            "mark_completed",
            "retry_job",
            "cancel_job",
            "cancel_ready_job",
            "skip_job",
        ] {
            let err = match *op_name {
                "start_generation" => q.start_generation(fake).unwrap_err(),
                "set_spoken_text" => q.set_spoken_text(fake, "err".into()).unwrap_err(),
                "mark_ready" => q.mark_ready(fake, "err".into()).unwrap_err(),
                "fail_generation" => q.fail_generation(fake, "err".into()).unwrap_err(),
                "mark_playing" => q.mark_playing(fake).unwrap_err(),
                "mark_completed" => q.mark_completed(fake).unwrap_err(),
                "retry_job" => q.retry_job(fake).unwrap_err(),
                "cancel_job" => q.cancel_job(fake).unwrap_err(),
                "cancel_ready_job" => q.cancel_ready_job(fake).unwrap_err(),
                "skip_job" => q.skip_job(fake).unwrap_err(),
                _ => unreachable!(),
            };
            assert!(
                text_err(&err).contains("not found"),
                "{op_name}: expected 'not found', got: {err}"
            );
        }
    }

    // ── state DTO ──

    #[test]
    fn state_dto_reflects_fifo_order() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        let id3 = q.submit("third", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.mark_ready(id1, "first".into()).unwrap();
        q.start_generation(id2).unwrap();
        q.fail_generation(id2, "err".into()).unwrap();

        let state = q.state();
        assert_eq!(state.jobs.len(), 3);
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[0].status, JobStatus::Ready);
        assert_eq!(state.jobs[1].job_id, id2);
        assert_eq!(state.jobs[1].status, JobStatus::Failed);
        assert_eq!(state.jobs[2].job_id, id3);
        assert_eq!(state.jobs[2].status, JobStatus::Queued);
    }

    #[test]
    fn state_dto_reflects_completed_statuses_after_all_completed() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.mark_ready(id1, "first".into()).unwrap();
        q.mark_playing(id1).unwrap();
        q.mark_completed(id1).unwrap();

        q.start_generation(id2).unwrap();
        q.mark_ready(id2, "second".into()).unwrap();
        q.mark_playing(id2).unwrap();
        q.mark_completed(id2).unwrap();

        let state = q.state();
        assert_eq!(state.jobs.len(), 2);
        assert_eq!(state.jobs[0].status, JobStatus::Completed);
        assert_eq!(state.jobs[1].status, JobStatus::Completed);
    }

    #[test]
    fn state_dto_empty_has_no_jobs() {
        let q = SpeechQueue::new();
        let state = q.state();
        assert!(state.jobs.is_empty());
    }

    #[test]
    fn state_dto_failed_job_preserves_error() {
        let mut q = SpeechQueue::new();
        let id = q.submit("only", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("err"));
    }

    // ── idempotent errors ──

    #[test]
    fn errors_do_not_mutate_state() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let before = serde_json::to_value(q.state()).unwrap();

        let _ = q.mark_ready(id, "text".into()); // expect error: not generating

        let after = serde_json::to_value(q.state()).unwrap();
        assert_eq!(before, after);
    }

    // ── timestamp type ──

    #[test]
    fn created_at_ms_is_i64_timestamp() {
        let before_ms = Utc::now().timestamp_millis();
        let mut q = SpeechQueue::new();
        q.submit("hello", snap()).unwrap();
        let after_ms = Utc::now().timestamp_millis();
        let ts = q.state().jobs[0].created_at_ms;
        assert!(ts >= before_ms);
        assert!(ts <= after_ms);
    }

    // ── failure paths followed by a queued job ──

    #[test]
    fn generation_failure_keeps_failed_status_error_and_claims_next_queued() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.fail_generation(id1, "provider error".into()).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("provider error"));

        assert_eq!(q.next_actionable(), Some(id2));

        let item = q
            .claim_next_generation()
            .unwrap()
            .expect("should claim second");
        assert_eq!(item.job_id, id2);
        assert_eq!(item.original_text, "second");

        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("provider error"));
        assert_eq!(state.jobs[1].status, JobStatus::Generating);
    }

    #[test]
    fn playback_failure_keeps_failed_status_error_and_claims_next_queued() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id1).unwrap();
        q.mark_ready(id1, "spoken first".into()).unwrap();
        q.fail_playback(id1, "device lost".into()).unwrap();

        let state = q.state();
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("device lost"));

        assert_eq!(q.next_actionable(), Some(id2));

        let item = q
            .claim_next_generation()
            .unwrap()
            .expect("should claim second");
        assert_eq!(item.job_id, id2);

        let state = q.state();
        assert_eq!(state.jobs[0].status, JobStatus::Failed);
        assert_eq!(state.jobs[0].error.as_deref(), Some("device lost"));
    }

    #[test]
    fn retry_of_only_failed_returns_to_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("only", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.retry_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
        assert_eq!(q.next_actionable(), Some(id));
    }

    #[test]
    fn cancel_of_only_failed_leaves_no_actionable() {
        let mut q = SpeechQueue::new();
        let id = q.submit("only", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert_eq!(q.next_actionable(), None);
    }

    // ── serialization regression ──

    #[test]
    fn serialization_regression() {
        // WorkItem deliberately has no Serialize/Deserialize derive.
        // Verify that state DTO JSON never contains snapshot or work_item.
        let mut q = SpeechQueue::new();
        q.submit("hello", snap()).unwrap();
        let json = serde_json::to_string(&q.state()).unwrap();
        assert!(
            !json.contains("snapshot"),
            "state JSON must not contain 'snapshot'"
        );
        assert!(
            !json.contains("work_item"),
            "state JSON must not contain 'work_item'"
        );
        assert!(
            !json.contains("WorkItem"),
            "state JSON must not contain 'WorkItem'"
        );
    }

    // ── has_job / get_status ──

    #[test]
    fn has_job_returns_true_for_existing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        assert!(q.has_job(id));
    }

    #[test]
    fn has_job_returns_false_for_unknown() {
        let q = SpeechQueue::new();
        assert!(!q.has_job(Uuid::new_v4()));
    }

    #[test]
    fn get_status_returns_correct_status() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Queued));
    }

    #[test]
    fn get_status_returns_none_for_unknown() {
        let q = SpeechQueue::new();
        assert_eq!(q.get_status(Uuid::new_v4()), None);
    }

    #[test]
    fn get_status_tracks_transitions() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Generating));
        q.mark_ready(id, "text".into()).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Ready));
        q.mark_playing(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Playing));
        q.mark_completed(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Completed));
    }

    #[test]
    fn state_json_excludes_snapshot_and_api_key() {
        let snapshot = Snapshot {
            provider: "test".into(),
            voice: "alloy".into(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(false, false),
            ai_enabled: true,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings {
                openai: AiOpenAiSettings {
                    api_key: Some("sk-test-sentinel-dont-expose".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
            tts_provider: TtsProvider::Local(
                crate::tts::local_http_server::LocalHttpServerTts::new(),
            ),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        };
        let mut q = SpeechQueue::new();
        q.submit("hello", snapshot).unwrap();
        let json = serde_json::to_string(&q.state()).unwrap();
        assert!(
            !json.contains("snapshot"),
            "state JSON must not contain snapshot field"
        );
        assert!(
            !json.contains("sk-test-sentinel-dont-expose"),
            "state JSON must not contain AI API key"
        );
    }

    // ── last_activity_at_ms ──

    #[test]
    fn submit_sets_last_activity_to_created_at() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let state = q.state();
        assert_eq!(
            state.jobs[0].last_activity_at_ms,
            state.jobs[0].created_at_ms
        );
        assert!(q.has_job(id));
    }

    #[test]
    fn retry_updates_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let before_ms = q.state().jobs[0].last_activity_at_ms;

        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        q.retry_job(id).unwrap();

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(after_ms > before_ms);
    }

    #[test]
    fn cancel_does_not_update_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let before_ms = q.state().jobs[0].last_activity_at_ms;

        std::thread::sleep(std::time::Duration::from_millis(5));
        q.cancel_job(id).unwrap();

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert_eq!(after_ms, before_ms);
    }

    #[test]
    fn skip_updates_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.fail_generation(id, "err".into()).unwrap();
        let before_ms = q.state().jobs[0].last_activity_at_ms;

        std::thread::sleep(std::time::Duration::from_millis(5));
        q.skip_job(id).unwrap();

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(after_ms > before_ms);
    }

    #[test]
    fn pipeline_transitions_do_not_update_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let initial = q.state().jobs[0].last_activity_at_ms;

        q.start_generation(id).unwrap();
        assert_eq!(q.state().jobs[0].last_activity_at_ms, initial);

        q.mark_ready(id, "text".into()).unwrap();
        assert_eq!(q.state().jobs[0].last_activity_at_ms, initial);

        q.mark_playing(id).unwrap();
        assert_eq!(q.state().jobs[0].last_activity_at_ms, initial);

        q.mark_completed(id).unwrap();
        assert_eq!(q.state().jobs[0].last_activity_at_ms, initial);
    }

    #[test]
    fn fail_generation_does_not_update_last_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let initial = q.state().jobs[0].last_activity_at_ms;

        q.fail_generation(id, "err".into()).unwrap();
        assert_eq!(q.state().jobs[0].last_activity_at_ms, initial);
    }

    #[test]
    fn touch_activity_updates_timestamp() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let before_ms = q.state().jobs[0].last_activity_at_ms;

        std::thread::sleep(std::time::Duration::from_millis(5));
        let touched = q.touch_activity(id);
        assert!(touched);

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(after_ms > before_ms);
    }

    #[test]
    fn touch_activity_unknown_id_returns_false() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        assert!(!q.touch_activity(fake));
        assert!(q.is_empty());
    }

    #[test]
    fn last_activity_preserves_fifo_order() {
        let mut q = SpeechQueue::new();
        let id1 = q.submit("first", snap()).unwrap();
        let id2 = q.submit("second", snap()).unwrap();
        let id3 = q.submit("third", snap()).unwrap();

        q.touch_activity(id3);

        let state = q.state();
        // FIFO order in jobs vector is unchanged
        assert_eq!(state.jobs[0].job_id, id1);
        assert_eq!(state.jobs[1].job_id, id2);
        assert_eq!(state.jobs[2].job_id, id3);
        // But last_activity of id3 is newer
        assert!(state.jobs[2].last_activity_at_ms > state.jobs[0].last_activity_at_ms);
    }

    #[test]
    fn touch_activity_is_strictly_monotonic() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();

        let first = q.state().jobs[0].last_activity_at_ms;

        q.touch_activity(id);
        let second = q.state().jobs[0].last_activity_at_ms;

        q.touch_activity(id);
        let third = q.state().jobs[0].last_activity_at_ms;

        assert!(second > first, "second bump must be > first");
        assert!(third > second, "third bump must be > second");
    }

    #[test]
    fn touch_activity_on_cancelled_with_spoken_text_preserves_status() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "cached audio".into()).unwrap();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert_eq!(
            q.state().jobs[0].spoken_text.as_deref(),
            Some("cached audio")
        );

        let before_ms = q.state().jobs[0].last_activity_at_ms;
        std::thread::sleep(std::time::Duration::from_millis(5));

        q.touch_activity(id);

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(
            after_ms > before_ms,
            "touch_activity must bump timestamp on cancelled job"
        );
        assert_eq!(
            q.state().jobs[0].status,
            JobStatus::Cancelled,
            "touch_activity must not change status"
        );
        assert_eq!(
            q.state().jobs[0].spoken_text.as_deref(),
            Some("cached audio")
        );
    }

    #[test]
    fn touch_activity_on_cancelled_queued_origin_preserves_status() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.cancel_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert!(q.state().jobs[0].spoken_text.is_none());

        let before_ms = q.state().jobs[0].last_activity_at_ms;
        std::thread::sleep(std::time::Duration::from_millis(5));

        q.touch_activity(id);

        let after_ms = q.state().jobs[0].last_activity_at_ms;
        assert!(after_ms > before_ms);
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
    }

    // ── cancel_generating_job ──

    #[test]
    fn cancel_generating_job_transitions_to_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Generating);

        q.cancel_generating_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);
        assert_eq!(q.state().jobs[0].error, None);
    }

    #[test]
    fn cancel_generating_job_preserves_no_error_for_restore() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.cancel_generating_job(id).unwrap();
        assert!(q.state().jobs[0].error.is_none());
        q.restore_cancelled_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
        assert_eq!(q.state().jobs[0].attempt, 1);
        assert_eq!(q.state().jobs[0].original_text, "hello");
    }

    #[test]
    fn cancel_generating_job_rejects_queued() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        let err = q.cancel_generating_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Queued);
    }

    #[test]
    fn cancel_generating_job_rejects_ready() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();
        let err = q.cancel_generating_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Ready);
    }

    #[test]
    fn cancel_generating_job_rejects_playing() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();
        q.mark_playing(id).unwrap();
        let err = q.cancel_generating_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
        assert_eq!(q.state().jobs[0].status, JobStatus::Playing);
    }

    #[test]
    fn cancel_generating_job_rejects_completed() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "text".into()).unwrap();
        q.mark_playing(id).unwrap();
        q.mark_completed(id).unwrap();
        let err = q.cancel_generating_job(id).unwrap_err();
        assert!(text_err(&err).contains("invalid transition"));
    }

    #[test]
    fn cancel_generating_job_does_not_update_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        let before = q.state().jobs[0].last_activity_at_ms;

        q.cancel_generating_job(id).unwrap();
        let after = q.state().jobs[0].last_activity_at_ms;
        assert_eq!(after, before);
    }

    #[test]
    fn cancel_generating_job_unknown_id_errors() {
        let mut q = SpeechQueue::new();
        let fake = Uuid::new_v4();
        let err = q.cancel_generating_job(fake).unwrap_err();
        assert!(text_err(&err).contains("not found"));
    }

    #[test]
    fn restore_generating_cancelled_preserves_snapshot_and_fifo() {
        let mut q = SpeechQueue::new();
        let id = q.submit("restore-me", snap()).unwrap();
        let _id2 = q.submit("second", snap()).unwrap();

        q.start_generation(id).unwrap();
        q.cancel_generating_job(id).unwrap();
        assert_eq!(q.state().jobs[0].status, JobStatus::Cancelled);

        q.restore_cancelled_job(id).unwrap();
        let state = q.state();
        assert_eq!(state.jobs[0].job_id, id);
        assert_eq!(state.jobs[0].original_text, "restore-me");
        assert_eq!(state.jobs[0].status, JobStatus::Queued);
        assert_eq!(state.jobs[0].attempt, 1);
        assert_eq!(state.jobs[0].spoken_text, None);
        assert_eq!(state.jobs[0].error, None);
        assert_eq!(state.jobs[1].job_id, _id2);
    }

    #[test]
    fn restore_generating_cancelled_updates_activity() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.cancel_generating_job(id).unwrap();
        let before = q.state().jobs[0].last_activity_at_ms;

        q.restore_cancelled_job(id).unwrap();
        let after = q.state().jobs[0].last_activity_at_ms;
        assert!(after > before);
    }

    // ── handoff guard for ready cancellation ──

    #[test]
    fn handoff_guard_absent_before_install_cancel_accepted() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();

        let guard = {
            let idx = q.jobs.iter().position(|j| j.job_id == id).unwrap();
            q.jobs[idx].handoff_guard.clone()
        };

        let g = guard.lock();
        assert_eq!(q.get_status(id), Some(JobStatus::Ready));
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Cancelled));
        drop(g);
    }

    #[test]
    fn handoff_guard_worker_rechecks_status_and_discards_when_cancelled() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();

        let guard = {
            let idx = q.jobs.iter().position(|j| j.job_id == id).unwrap();
            q.jobs[idx].handoff_guard.clone()
        };

        let g = guard.lock();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Cancelled));
        drop(g);

        let g = guard.lock();
        let status = q.get_status(id);
        assert_eq!(status, Some(JobStatus::Cancelled));
        drop(g);
    }

    #[test]
    fn handoff_guard_cancel_sees_cancelled_no_double_mutation() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();
        q.mark_ready(id, "spoken".into()).unwrap();

        let guard = {
            let idx = q.jobs.iter().position(|j| j.job_id == id).unwrap();
            q.jobs[idx].handoff_guard.clone()
        };

        let g = guard.lock();
        q.cancel_ready_job(id).unwrap();
        assert_eq!(q.get_status(id), Some(JobStatus::Cancelled));
        assert!(q.cancel_ready_job(id).is_err());
        drop(g);
    }

    #[test]
    fn handoff_guard_reset_on_restore() {
        let mut q = SpeechQueue::new();
        let id = q.submit("hello", snap()).unwrap();
        q.start_generation(id).unwrap();

        let guard1 = {
            let idx = q.jobs.iter().position(|j| j.job_id == id).unwrap();
            q.jobs[idx].handoff_guard.clone()
        };

        q.cancel_generating_job(id).unwrap();

        q.restore_cancelled_job(id).unwrap();
        let guard2 = {
            let idx = q.jobs.iter().position(|j| j.job_id == id).unwrap();
            q.jobs[idx].handoff_guard.clone()
        };

        assert!(!Arc::ptr_eq(&guard1, &guard2));
    }

    // ── event DTO fixtures ──

    pub(crate) fn build_populated_speech_queue_state_dto() -> SpeechQueueStateDto {
        let snapshot = Snapshot {
            provider: "test-provider".into(),
            voice: "test-voice".into(),
            source: SubmissionSource::Editor,
            delivery: DeliveryPolicy::editor(true, false),
            ai_enabled: true,
            number_conversion_enabled: true,
            audio_effects: AudioEffectsSettings::default(),
            dsp: DspSettings::default(),
            audio: AudioSettings::default(),
            ai: AiSettings::default(),
            tts_provider: TtsProvider::Local(
                crate::tts::local_http_server::LocalHttpServerTts::new(),
            ),
            preprocessor: None,
            network_settings: NetworkSettings::default(),
            accentor_runtime: None,
        };

        let mut q = SpeechQueue::new();

        let job1 = SpeechJob::test_job(
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            "Hello world".into(),
            Some("Processed hello".into()),
            JobStatus::Completed,
            None,
            1,
            1000000000000i64,
            1000000000000i64,
            snapshot.clone(),
        );
        q.push_job(job1);

        let job2 = SpeechJob::test_job(
            Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            "Goodbye".into(),
            None,
            JobStatus::Failed,
            Some("TTS provider timeout".into()),
            1,
            1000000000000i64,
            1000000000000i64,
            snapshot,
        );
        q.push_job(job2);

        q.state()
    }

    pub(crate) fn build_empty_speech_queue_state_dto() -> SpeechQueueStateDto {
        SpeechQueue::new().state()
    }

    #[test]
    fn speech_contract_event_fixture_populated_is_current() {
        let dto = build_populated_speech_queue_state_dto();
        let current = serde_json::to_value(&dto).expect("serialize");
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/speech/speech-queue-populated.json"
        );
        let on_disk = std::fs::read_to_string(fixture_path).unwrap_or_else(|_| {
            panic!(
                "Fixture 'speech-queue-populated.json' missing — regenerate with \
                 `cargo test speech_contract_fixtures_regenerate -- --ignored`"
            );
        });
        let fixture: serde_json::Value = serde_json::from_str(&on_disk).expect("parse fixture");
        assert_eq!(
            current, fixture,
            "Fixture 'speech-queue-populated.json' is stale — regenerate with \
             `cargo test speech_contract_fixtures_regenerate -- --ignored`",
        );
    }

    #[test]
    fn speech_contract_event_fixture_empty_is_current() {
        let dto = build_empty_speech_queue_state_dto();
        let current = serde_json::to_value(&dto).expect("serialize");
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/speech/speech-queue-empty.json"
        );
        let on_disk = std::fs::read_to_string(fixture_path).unwrap_or_else(|_| {
            panic!(
                "Fixture 'speech-queue-empty.json' missing — regenerate with \
                 `cargo test speech_contract_fixtures_regenerate -- --ignored`"
            )
        });
        let fixture: serde_json::Value = serde_json::from_str(&on_disk).expect("parse fixture");
        assert_eq!(
            current, fixture,
            "Fixture 'speech-queue-empty.json' is stale — regenerate with \
             `cargo test speech_contract_fixtures_regenerate -- --ignored`"
        );
    }

    /// Regenerates all speech queue event fixtures. Excluded from standard test runs.
    #[test]
    #[ignore]
    fn speech_contract_fixtures_regenerate() {
        let fixture_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/contract-fixtures/speech"
        );

        std::fs::create_dir_all(fixture_dir).expect("create fixture dir");

        let populated = build_populated_speech_queue_state_dto();
        let populated_json = serde_json::to_string_pretty(&populated).expect("serialize populated");
        let populated_path = format!("{}/speech-queue-populated.json", fixture_dir);
        std::fs::write(&populated_path, &populated_json).expect("write populated fixture");
        eprintln!("Fixture written: {}", populated_path);

        let empty = build_empty_speech_queue_state_dto();
        let empty_json = serde_json::to_string_pretty(&empty).expect("serialize empty");
        let empty_path = format!("{}/speech-queue-empty.json", fixture_dir);
        std::fs::write(&empty_path, &empty_json).expect("write empty fixture");
        eprintln!("Fixture written: {}", empty_path);
    }
}
