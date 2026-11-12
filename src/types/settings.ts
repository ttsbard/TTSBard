/**
 * Application settings types
 *
 * These types match the Rust DTOs for unified settings loading.
 */

import type { SoundBinding } from '../types'
import type { EditorRoute } from '../components/editor/routeDecode'

// ============================================================================
// Hotkey Settings Types
// ============================================================================

export type HotkeyModifier = 'ctrl' | 'shift' | 'alt' | 'super'

export interface HotkeyDto {
  modifiers: HotkeyModifier[]
  key: string
  /**
   * Per-hotkey switch. The backend omits it for the default `true`, so a
   * missing value means enabled; `false` keeps the combination stored but
   * inactive.
   */
  enabled?: boolean
}

export interface EditorHotkeySettingsDto {
  edit_word: HotkeyDto
  submit_continue: HotkeyDto
  submit_keep_text: HotkeyDto
  submit_keep_focus: HotkeyDto
  next_spelling_error: HotkeyDto
  previous_spelling_error: HotkeyDto
  next_tab: HotkeyDto
  previous_tab: HotkeyDto
  cycle_route: HotkeyDto
  toggle_typing: HotkeyDto
  cycle_quick_mode: HotkeyDto
  toggle_history: HotkeyDto
  accent_homographs: HotkeyDto
  approve_next_incoming: HotkeyDto
  edit_next_incoming: HotkeyDto
}

export interface HotkeySettingsDto {
  main_window: HotkeyDto
  sound_panel: HotkeyDto
  playback_pause: HotkeyDto
  playback_stop: HotkeyDto
  playback_repeat: HotkeyDto
  playback_control_window: HotkeyDto
  return_previous_window: HotkeyDto
  toggle_minimal_mode: HotkeyDto
  ocr_capture: HotkeyDto
  editor: EditorHotkeySettingsDto
}

// ============================================================================
// TTS Settings Types
// ============================================================================

// Rust enum uses #[serde(rename_all = "lowercase")]
// So JSON returns: "openai", "silero", "local", "fish", "elevenlabs"
export const TtsProviderType = {
  OpenAi: 'openai',
  Silero: 'silero',
  Local: 'local',
  Fish: 'fish',
  ElevenLabs: 'elevenlabs'
} as const

export type TtsProviderType = (typeof TtsProviderType)[keyof typeof TtsProviderType]

export interface OpenAiSettingsDto {
  api_key: string | null
  voice: string
  proxy_host: string | null
  proxy_port: number | null
  use_proxy: boolean
}

export interface LocalTtsSettingsDto {
  url: string
}

/// Голосовая модель Fish Audio
export interface VoiceModel {
  id: string
  title: string
  description: string | null
  cover_image: string | null
  languages: string[]
  author_nickname: string | null
}

export interface FishAudioSettingsDto {
  api_key: string | null
  voices: VoiceModel[]
  reference_id: string
  format: string
  temperature: number
  sample_rate: number
  use_proxy: boolean
}

/** Atomic payload for `save_fish_audio_connection_settings`. */
export interface FishAudioConnectionSettingsInput {
  apiKey: string
  format: string
  temperature: number
  sampleRate: number
}

/// Голосовой дескриптор ElevenLabs
export type ElevenLabsVoiceClassification = 'default' | 'library'

export interface ElevenLabsVoice {
  voice_id: string
  name: string
  category: string | null
  labels: string[]
  preview_url: string | null
  /** `default`/`library` for the two displayed partitions; `null`/absent for an unmarked personal/workspace voice. */
  classification?: ElevenLabsVoiceClassification | null
}

/** Дескриптор модели ElevenLabs (account-backed catalog entry). */
export interface ElevenLabsModel {
  model_id: string
  name: string
  can_use_style: boolean
  can_use_speaker_boost: boolean
}

export interface ElevenLabsSettingsDto {
  api_key: string | null
  voice_id: string
  voices: ElevenLabsVoice[]
  models: ElevenLabsModel[]
  model_id: string
  output_format: string
  stability: number
  similarity_boost: number
  style: number
  use_speaker_boost: boolean
  use_proxy: boolean
}

/** Atomic payload for `save_elevenlabs_generation_settings`. */
export interface ElevenLabsGenerationSettingsInput {
  modelId: string
  outputFormat: string
  stability: number
  similarityBoost: number
  style: number
  useSpeakerBoost: boolean
}

export interface TelegramTtsSettingsDto {
  api_id: number | null
  proxy_mode: string
  voices: VoiceCode[]
  current_voice_id: string
  synthesis_response_timeout_ms: number
  download_retry_delay_ms: number
}

export interface VoiceCode {
  id: string
  description?: string
}

export interface Socks5SettingsDto {
  proxy_url: string | null
}

export interface MtProxySettingsDto {
  host: string | null
  port: number
  secret: string | null
  dc_id: number | null
}

export interface NetworkSettingsDto {
  proxy: Socks5SettingsDto
  mtproxy: MtProxySettingsDto
}

export interface TtsProviderInfoDto {
  id: string
  display_name: string
  kind: string
  active: boolean
  runtime_status?: string
}

export interface TtsSettingsDto {
  provider: TtsProviderType
  provider_id: string | null
  providers: TtsProviderInfoDto[]
  visible_provider_ids?: string[]
  openai: OpenAiSettingsDto
  local: LocalTtsSettingsDto
  fish: FishAudioSettingsDto
  elevenlabs: ElevenLabsSettingsDto
  telegram: TelegramTtsSettingsDto
  network: NetworkSettingsDto
}

// ============================================================================
// Legacy Proxy Settings Types (deprecated, use NetworkSettingsDto)
// ============================================================================

export type ProxyTypeDto = 'Socks5' | 'Socks4' | 'Http'

export interface ProxySettingsDto {
  proxy_url?: string
  proxy_type: ProxyTypeDto
}

// ============================================================================
// WebView Settings Types
// ============================================================================

export interface WebViewSettingsDto {
  enabled: boolean
  start_on_boot: boolean
  port: number
  access_token: string | null
  upnp_enabled: boolean
  send_original_text: boolean
}

// ============================================================================
// Twitch Settings Types
// ============================================================================

export type TwitchMode = 'irc' | 'api'

export interface TwitchSettingsDto {
  mode: TwitchMode
  enabled: boolean
  username: string
  token: string
  channel: string
  start_on_boot: boolean
  send_original_text: boolean
}

// ============================================================================
// Audio Settings Types
// ============================================================================

export interface AudioSettingsDto {
  output_format: 'default' | 'i32'
  speaker_device: string | null
  speaker_enabled: boolean
  speaker_volume: number
  virtual_mic_device: string | null
  virtual_mic_volume: number
}

// ============================================================================
// Audio Effects Settings Types
// ============================================================================

export interface AudioEffectsSettingsDto {
  enabled: boolean
  pitch: number  // -100 to +100 → -12..+12 semitones
  speed: number  // -100 to +100 → 0.75..1.50 tempo factor
  volume: number // 0 to 200
  enhance_enabled: boolean // DeepFilterNet noise suppression
  enhance_atten_db: number // 5 to 30 dB (attenuation limit)
  formant_preserved: boolean // Signalsmith formant correction (default: true)
  boundary_cleanup_enabled: boolean // per-phrase boundary cleanup (default: true)
}

// ============================================================================
// DSP Post-Processing Settings Types
// ============================================================================

export interface DspEqBandSettingsDto {
  enabled: boolean
  frequency_hz: number
  gain_db: number
  q: number
}

export interface DspEqSettingsDto {
  enabled: boolean
  low_cut_enabled: boolean
  low_cut_hz: number
  low_cut_slope_db: number
  bands: DspEqBandSettingsDto[]
  high_shelf_enabled: boolean
  high_shelf_hz: number
  high_shelf_gain_db: number
}

export interface DspCompressorSettingsDto {
  enabled: boolean
  threshold_db: number
  ratio: number
  attack_ms: number
  release_ms: number
  knee_db: number
  makeup_db: number
}

export interface DspLimiterSettingsDto {
  enabled: boolean
  ceiling_db: number
  release_ms: number
}

export interface DspSettingsDto {
  eq: DspEqSettingsDto
  compressor: DspCompressorSettingsDto
  limiter: DspLimiterSettingsDto
}

// ============================================================================
// Logging Settings Types
// ============================================================================

export interface LoggingSettingsDto {
  enabled: boolean
  level: string
  module_levels: Record<string, string>
}

// ============================================================================
// Windows Settings Types
// ============================================================================

export type CompactView = 'compact' | 'mono'

export interface MainWindowSettingsDto {
  x: number | null
  y: number | null
  custom_background: boolean
  opacity: number
  bg_color: string
  custom_opacity: boolean
  opacity_compact_only: boolean
  compact_width: number
  compact_height: number
  compact_view: CompactView
  hide_extra_window_buttons: boolean
}

export interface SoundPanelWindowSettingsDto {
  x: number | null
  y: number | null
  opacity: number
  bg_color: string
  clickthrough: boolean
  stay_visible: boolean
  hide_on_blur: boolean
  appearance_source: string
}

export interface PlaybackWindowSettingsDto {
  x: number | null
  y: number | null
  opacity: number
  bg_color: string
  appearance_source: string
}

export interface GlobalSettingsDto {
  exclude_from_capture: boolean
}

export interface WindowsSettingsDto {
  global: GlobalSettingsDto
  main: MainWindowSettingsDto
  soundpanel: SoundPanelWindowSettingsDto
  playback: PlaybackWindowSettingsDto
}

// ============================================================================
// General Settings Types
// ============================================================================

export type Theme = 'dark' | 'light'

export interface GeneralSettingsDto {
  hotkey_enabled: boolean
  theme?: Theme
  ui_language: string
  /** Interface font family: `'default'` keeps the built-in CSS stack. */
  ui_font_family: string
  /** Interface font size: the `html` rem base of the main window in px. */
  ui_font_size_px: number
  show_playback_on_start: boolean
  start_compact: boolean
  hide_on_minimize: boolean
}

// ============================================================================
// Preprocessor Settings Types
// ============================================================================

export interface PreprocessorSettingsDto {
  enabled: boolean
  replacements_count: number
}

// ============================================================================
// Editor Settings Types
// ============================================================================

export type QuickEditorMode = 'disabled' | 'collapse' | 'return_focus'

/** Editor typography font family: a built-in choice or an installed Windows family name. */
export type EditorFontFamily = string

/** Interface typography font family: a built-in choice or an installed Windows family name. */
export type InterfaceFontFamily = string

export interface ContextualRuAccentSettingsDto {
  enabled: boolean
  python_executable: string | null
  model_dir: string | null
  model: string
}

export interface HomographAccentorSettingsDto {
  enabled: boolean
  accentor_pack_id: string | null
  load_on_start: boolean
  contextual?: ContextualRuAccentSettingsDto
}

/** Diagnostic runtime status of a discovered RUAccent pack. */
export type HomographAccentorPackStatus = 'not_loaded' | 'loading' | 'ready' | 'failed'

/** DTO for a discovered local RUAccent pack (no filesystem paths). */
export interface HomographAccentorPackDto {
  id: string
  display_name: string
  runtime_version: string
  runtime_status: HomographAccentorPackStatus
}

export interface EditorSettingsDto {
  quick: QuickEditorMode
  ai: boolean
  ai_completion: boolean
  autocomplete_enabled: boolean
  number_conversion_enabled: boolean
  spellcheck_enabled: boolean
  spellcheck_source: 'online' | 'offline'
  editor_height: number
  typing_idle_timeout_ms: number
  typing_enabled: boolean
  default_route: EditorRoute
  keep_text_after_send: boolean
  font_family: EditorFontFamily
  font_size_px: number
  homograph_accentor: HomographAccentorSettingsDto
}

// ============================================================================
// AI Settings Types
// ============================================================================

export const AiProviderType = {
  OpenAi: 'openai',
  ZAi: 'zai',  // Z.ai (capital Z)
  DeepSeek: 'deepseek',
  Custom: 'custom',
} as const

export type AiProviderType = (typeof AiProviderType)[keyof typeof AiProviderType]

export interface AiOpenAiSettingsDto {
  api_key: string | null
  use_proxy: boolean
  model: string
}

export interface AiZAiSettingsDto {
  url: string | null
  api_key: string | null
  model: string
}

export interface AiDeepSeekSettingsDto {
  api_key: string | null
  use_proxy: boolean
  model: string
}

export interface AiCustomSettingsDto {
  url: string | null
  api_key: string | null
  use_proxy: boolean
  model: string
}

// Z.ai (Anthropic-compatible AI provider)

export interface AiSettingsDto {
  provider: AiProviderType
  openai: AiOpenAiSettingsDto
  zai: AiZAiSettingsDto
  deepseek: AiDeepSeekSettingsDto
  custom: AiCustomSettingsDto
  prompt: string
  timeout: number
}

// ============================================================================
// VTube Studio Settings Types
// ============================================================================

export type VTubeStudioTypingMode = 'Event' | 'Hotkeys' | 'Item'

export interface VTubeStudioTypingActionDto {
  outputMode: VTubeStudioTypingMode
  parameterName: string
  startHotkeyId: string
  stopHotkeyId: string
  startHotkeyName: string
  stopHotkeyName: string
  itemFileName: string
  itemType: string
}

export interface VtsHotkeyInfoDto {
  hotkeyID: string
  name: string
  type: string
  description: string
}

/**
 * VTube Studio settings DTO (safe — no token exposed to frontend)
 */
export interface VTubeStudioSettingsDto {
  enabled: boolean
  /** VTS Plugin API server address: IP or computer name, no scheme and no port. */
  host: string
  port: number
  start_on_boot: boolean
  typingAction: VTubeStudioTypingActionDto
}

/**
 * VTube Studio item status (discriminated union).
 *
 * Mirrors the tagged Rust enum `VTubeStudioItemStatus`:
 *   - status: "Inactive" — no active item action.
 *   - status: "Ready" — item is resolved, animated to ready state.
 *   - status: "Missing" — configured filename not found in scene.
 *   - status: "Ambiguous" — multiple instances share the filename.
 *   - status: "Unsupported" — item type is not usable for typing action.
 *   - status: "Error" — an unexpected error occurred during resolution.
 */
export type VTubeStudioItemStatus =
  | { status: 'Inactive' }
  | { status: 'Ready'; fileName: string; vtsType: string }
  | { status: 'Missing'; fileName: string }
  | { status: 'Ambiguous'; fileName: string; matchCount: number }
  | { status: 'Unsupported'; fileName: string; vtsType: string }
  | { status: 'Error'; fileName: string; message: string }

/**
 * Aggregate scene item record (no instanceID).
 */
export interface SceneItemRecord {
  fileName: string
  itemType: string
  supported: boolean
  duplicateCount: number
}

// ============================================================================
// OCR Settings Types
// ============================================================================

/**
 * Discriminator of a persisted capture target. Mirrors the Rust `CaptureTarget`
 * tag: `all`, `primary` or a specific `monitor` (identified by its persistent
 * Windows device path, never an index or temporary handle).
 */
export type CaptureTargetType = 'all' | 'primary' | 'monitor'

/**
 * Persisted capture target DTO.
 *
 * The Rust enum is internally tagged by `type`:
 * `{"type":"all"}`, `{"type":"primary"}` and
 * `{"type":"monitor","devicePath":"..."}`. `devicePath` is present only (and
 * required) on the `monitor` variant.
 */
export interface CaptureTargetDto {
  type: CaptureTargetType
  devicePath?: string
}

/**
 * Persisted one-shot screen OCR settings DTO.
 *
 * Mirrors the Rust `OcrSettings`/`OcrSettingsDto`: only the persisted desired
 * state — `enabled`, nullable `model_id` and the capture target — never runtime
 * status, capture paths, engine internals or hotkeys. Legacy settings without a
 * `capture_target` deserialize to the default `{"type":"all"}`.
 */
export interface OcrSettingsDto {
  enabled: boolean
  model_id: string | null
  capture_target: CaptureTargetDto
}

// ============================================================================
// Main App Settings DTO
// ============================================================================

/**
 * All application settings in a single DTO
 * This is the response from get_all_app_settings command
 */
export interface AppSettingsDto {
  startup_errors?: string[]
  storage: { data_dir: string | null; audio_cache_dir: string | null }
  notifications?: string[]
  tts: TtsSettingsDto
  webview: WebViewSettingsDto
  twitch: TwitchSettingsDto
  windows: WindowsSettingsDto
  audio: AudioSettingsDto
  audio_effects: AudioEffectsSettingsDto
  dsp: DspSettingsDto
  general: GeneralSettingsDto
  logging: LoggingSettingsDto
  preprocessor: PreprocessorSettingsDto
  soundpanel_bindings: SoundBinding[]
  editor: EditorSettingsDto
  ocr: OcrSettingsDto
  ai: AiSettingsDto
  hotkeys: HotkeySettingsDto
  vtube_studio: VTubeStudioSettingsDto
  vrchat: VrchatSettingsDto
}

export interface VrchatSettingsDto {
  enabled: boolean
  start_on_boot: boolean
  host: string
  port: number
}

// ============================================================================
// Tauri Command Types
// ============================================================================

export type AppSettingsCommand = 'get_all_app_settings' | 'is_backend_ready' | 'confirm_backend_ready'

// ============================================================================
// Injection Key
// ============================================================================

import { InjectionKey, Ref } from 'vue'

export interface AppSettingsContext {
  settings: Ref<AppSettingsDto | null>
  isLoading: Ref<boolean>
  error: Ref<string | null>
  reload: () => Promise<void>
  cleanup?: () => void
}

export const APP_SETTINGS_KEY: InjectionKey<AppSettingsContext> = Symbol('app-settings')

export const SETTINGS_CHANGED_EVENT = 'settings-changed'
