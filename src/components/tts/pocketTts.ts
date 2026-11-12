/**
 * Types and pure helpers for the Pocket TTS card.
 *
 * Mirrors the backend DTO contract from task 001 (`PocketModelDto`,
 * `PocketReferenceCheckDto`, `PocketVoiceInput`) and keeps the card's pure
 * decision logic free of Vue, IPC and DOM so it can be unit-tested.
 */

export type PocketVoiceMode = 'reference_wav' | 'voice_state'

export type PocketReadiness =
  | 'ready'
  | 'needs_reference'
  | 'reference_missing'
  | 'reference_invalid'
  | 'voice_missing'
  | 'package_invalid'

export interface PocketVoiceDto {
  id: string
  display_name: string
  available: boolean
}

export type PocketSelection =
  | { kind: 'reference_wav'; path: string; file_name: string }
  | { kind: 'voice_state'; voice_id: string; display_name: string }

export interface PocketModelDto {
  id: string
  display_name: string
  language: string
  format: string
  format_version: number
  package_id: string
  voice_mode: PocketVoiceMode
  voices: PocketVoiceDto[]
  selection: PocketSelection | null
  readiness: PocketReadiness
  reference_duration_seconds: number | null
  error_code: string | null
  loaded: boolean
}

export type PocketVoiceInput =
  | { kind: 'reference_wav'; path: string }
  | { kind: 'voice_state'; voice_id: string }

export type PocketReferenceStatus = 'ready' | 'reference_missing' | 'reference_invalid'

export interface PocketReferenceCheckDto {
  status: PocketReferenceStatus
  error_code: string | null
  file_name: string | null
  duration_seconds: number | null
  sample_rate: number | null
  channels: number | null
}

export interface PocketLanguageView {
  /** Short badge shown in the model list, e.g. `RU`. */
  code: string
  /** Translation key for the full language name, or null for unknown tags. */
  labelKey: string | null
  /** Original descriptor value for the tooltip of unknown languages. */
  raw: string
}

export type PocketStatusKind = 'ready' | 'warning' | 'error'

/** Map a backend readiness value to a translated language badge descriptor. */
export function pocketLanguageView(language: string): PocketLanguageView {
  const raw = (language ?? '').trim()
  const normalized = raw.toLowerCase()
  if (normalized === 'ru' || normalized.startsWith('ru-') || normalized === 'rus') {
    return { code: 'RU', labelKey: 'tts.pocket.language.ru', raw }
  }
  if (normalized === 'en' || normalized.startsWith('en-') || normalized === 'eng') {
    return { code: 'EN', labelKey: 'tts.pocket.language.en', raw }
  }
  return {
    code: raw ? raw.toUpperCase().slice(0, 8) : '—',
    labelKey: null,
    raw,
  }
}

/** True when the package accepts a user WAV reference. */
export function isPocketReferenceModel(model: PocketModelDto): boolean {
  return model.voice_mode === 'reference_wav'
}

/** Selected reference for a model, or null. */
export function pocketSelectedReference(
  model: PocketModelDto,
): Extract<PocketSelection, { kind: 'reference_wav' }> | null {
  return model.selection?.kind === 'reference_wav' ? model.selection : null
}

/** Selected ready voice for a model, or null. */
export function pocketSelectedVoice(
  model: PocketModelDto,
): Extract<PocketSelection, { kind: 'voice_state' }> | null {
  return model.selection?.kind === 'voice_state' ? model.selection : null
}

/** Translation key for the short readiness label in the model list. */
export function pocketStatusLabelKey(readiness: PocketReadiness): string {
  switch (readiness) {
    case 'ready':
      return 'tts.pocket.status.ready'
    case 'needs_reference':
      return 'tts.pocket.status.needs_reference'
    case 'reference_missing':
      return 'tts.pocket.status.reference_missing'
    case 'reference_invalid':
      return 'tts.pocket.status.reference_invalid'
    case 'voice_missing':
      return 'tts.pocket.status.voice_missing'
    case 'package_invalid':
      return 'tts.pocket.status.package_invalid'
    default:
      return 'tts.pocket.status.unknown'
  }
}

/** Translation key for the actionable readiness hint, or null when ready. */
export function pocketStatusHintKey(readiness: PocketReadiness): string | null {
  switch (readiness) {
    case 'needs_reference':
      return 'tts.pocket.hint.needs_reference'
    case 'reference_missing':
      return 'tts.pocket.hint.reference_missing'
    case 'reference_invalid':
      return 'tts.pocket.hint.reference_invalid'
    case 'voice_missing':
      return 'tts.pocket.hint.voice_missing'
    case 'package_invalid':
      return 'tts.pocket.hint.package_invalid'
    default:
      return null
  }
}

/** Visual tone of a readiness label. */
export function pocketStatusKind(readiness: PocketReadiness): PocketStatusKind {
  if (readiness === 'ready') return 'ready'
  if (readiness === 'needs_reference') return 'warning'
  return 'error'
}

/**
 * Pick the model whose details should be shown. The persisted active model has
 * priority, then the current focus while it still exists, then the first model.
 */
export function nextPocketFocusId(
  models: PocketModelDto[],
  activeModelId: string | null | undefined,
  currentFocusId: string | null | undefined,
): string | null {
  if (activeModelId && models.some((model) => model.id === activeModelId)) {
    return activeModelId
  }
  if (currentFocusId && models.some((model) => model.id === currentFocusId)) {
    return currentFocusId
  }
  return models[0]?.id ?? null
}

/** Replace one model by stable ID without mutating the snapshot array. */
export function replacePocketModel(
  models: PocketModelDto[],
  updated: PocketModelDto,
): PocketModelDto[] {
  const index = models.findIndex((model) => model.id === updated.id)
  if (index < 0) return models
  const next = models.slice()
  next[index] = updated
  return next
}

/** True when the model has a validated voice and can run the synthesis check. */
export function canTestPocketModel(
  model: PocketModelDto | null | undefined,
): boolean {
  return !!model && model.readiness === 'ready'
}

const RUSSIAN_TEST_PHRASES = [
  'Проверка синтеза прошла успешно.',
  'Эта фраза создана для проверки голоса.',
  'Быстрая лиса прыгает через ленивую собаку.',
  'Сегодня хорошая погода для новой записи.',
] as const

const ENGLISH_TEST_PHRASES = [
  'Synthesis check completed successfully.',
  'This sentence was created to test the voice.',
  'The quick brown fox jumps over the lazy dog.',
  'Today is a good day for a new recording.',
] as const

function normalizeTestLanguage(language: string): 'ru' | 'en' {
  const normalized = (language ?? '').trim().toLowerCase()
  return normalized === 'ru' || normalized.startsWith('ru-') || normalized === 'rus' ? 'ru' : 'en'
}

/** Phrases used by the new-sentence synthesis check, in the model language. */
export function pocketTestPhrases(language: string): readonly string[] {
  return normalizeTestLanguage(language) === 'ru' ? RUSSIAN_TEST_PHRASES : ENGLISH_TEST_PHRASES
}

/**
 * Pick a phrase for `submit_speech`. Consecutive checks must not repeat the
 * previous phrase, so each press produces a genuinely new sentence.
 */
export function pickPocketTestPhrase(
  language: string,
  previous: string | null | undefined,
  random: () => number = Math.random,
): string {
  const phrases = pocketTestPhrases(language)
  if (phrases.length === 1) return phrases[0]
  const normalized = Math.min(Math.max(random(), 0), 0.999999)
  let index = Math.floor(normalized * phrases.length)
  if (phrases[index] === previous) index = (index + 1) % phrases.length
  return phrases[index]
}

/** Stable visible label for a language badge. */
export function pocketLanguageLabel(
  view: PocketLanguageView,
  translate: (key: string) => string,
): string {
  if (view.labelKey) return translate(view.labelKey)
  return view.raw || view.code
}
