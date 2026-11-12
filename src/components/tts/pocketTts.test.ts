import { describe, expect, it } from 'vitest'
import {
  canTestPocketModel,
  isPocketReferenceModel,
  nextPocketFocusId,
  pickPocketTestPhrase,
  pocketLanguageView,
  pocketSelectedReference,
  pocketSelectedVoice,
  pocketStatusHintKey,
  pocketStatusKind,
  pocketStatusLabelKey,
  pocketTestPhrases,
  replacePocketModel,
  type PocketModelDto,
} from './pocketTts'

function model(overrides: Partial<PocketModelDto> = {}): PocketModelDto {
  return {
    id: 'local-pocket:shtorm',
    display_name: 'Shtorm RU v2 Fast',
    language: 'ru',
    format: 'pocket-tts-onnx',
    format_version: 1,
    package_id: 'shtorm',
    voice_mode: 'reference_wav',
    voices: [],
    selection: null,
    reference_duration_seconds: null,
    readiness: 'needs_reference',
    error_code: 'pocket.reference_required',
    loaded: false,
    ...overrides,
  }
}

describe('Pocket TTS card helpers', () => {
  it('maps known and unknown language tags to a badge and label key', () => {
    expect(pocketLanguageView('ru')).toMatchObject({ code: 'RU', labelKey: 'tts.pocket.language.ru' })
    expect(pocketLanguageView('ru-RU')).toMatchObject({ code: 'RU' })
    expect(pocketLanguageView('en')).toMatchObject({ code: 'EN', labelKey: 'tts.pocket.language.en' })
    expect(pocketLanguageView('en-US')).toMatchObject({ code: 'EN' })
    expect(pocketLanguageView('de')).toMatchObject({ code: 'DE', labelKey: null })
    expect(pocketLanguageView('')).toMatchObject({ code: '—', labelKey: null })
  })

  it('distinguishes reference and ready-voice packages', () => {
    expect(isPocketReferenceModel(model())).toBe(true)
    const kyutai = model({
      id: 'local-pocket:kyutai',
      voice_mode: 'voice_state',
      voices: [{ id: 'alba', display_name: 'Alba', available: true }],
      selection: { kind: 'voice_state', voice_id: 'alba', display_name: 'Alba' },
      readiness: 'ready',
      error_code: null,
    })
    expect(isPocketReferenceModel(kyutai)).toBe(false)
    expect(pocketSelectedReference(kyutai)).toBeNull()
    expect(pocketSelectedVoice(kyutai)).toMatchObject({ voice_id: 'alba' })
  })

  it('maps readiness to a status label, hint and tone', () => {
    expect(pocketStatusLabelKey('ready')).toBe('tts.pocket.status.ready')
    expect(pocketStatusLabelKey('needs_reference')).toBe('tts.pocket.status.needs_reference')
    expect(pocketStatusLabelKey('reference_missing')).toBe('tts.pocket.status.reference_missing')
    expect(pocketStatusLabelKey('reference_invalid')).toBe('tts.pocket.status.reference_invalid')
    expect(pocketStatusLabelKey('voice_missing')).toBe('tts.pocket.status.voice_missing')
    expect(pocketStatusLabelKey('package_invalid')).toBe('tts.pocket.status.package_invalid')
    expect(pocketStatusHintKey('ready')).toBeNull()
    expect(pocketStatusHintKey('reference_missing')).toBe('tts.pocket.hint.reference_missing')
    expect(pocketStatusKind('ready')).toBe('ready')
    expect(pocketStatusKind('needs_reference')).toBe('warning')
    expect(pocketStatusKind('reference_invalid')).toBe('error')
  })

  it('keeps the active model focused, then the current focus, then the first model', () => {
    const first = model({ id: 'local-pocket:a' })
    const second = model({ id: 'local-pocket:b' })
    const models = [first, second]
    expect(nextPocketFocusId(models, second.id, first.id)).toBe(second.id)
    expect(nextPocketFocusId(models, null, second.id)).toBe(second.id)
    expect(nextPocketFocusId(models, null, null)).toBe(first.id)
    expect(nextPocketFocusId(models, 'local-pocket:gone', 'local-pocket:gone')).toBe(first.id)
    expect(nextPocketFocusId([], second.id, first.id)).toBeNull()
  })

  it('replaces only the matching model and never mutates the snapshot', () => {
    const first = model({ id: 'local-pocket:a' })
    const second = model({ id: 'local-pocket:b' })
    const snapshot = [first, second]
    const updated = { ...first, display_name: 'Updated', readiness: 'ready' as const }
    const next = replacePocketModel(snapshot, updated)
    expect(next).not.toBe(snapshot)
    expect(snapshot[0].display_name).toBe('Shtorm RU v2 Fast')
    expect(next[0].display_name).toBe('Updated')
    expect(next[1]).toBe(second)
    expect(replacePocketModel(snapshot, model({ id: 'local-pocket:unknown' }))).toBe(snapshot)
  })

  it('enables the synthesis check for a ready model with a validated voice', () => {
    const ready = model({ readiness: 'ready' })
    expect(canTestPocketModel(ready)).toBe(true)
    expect(canTestPocketModel(model({ readiness: 'reference_missing' }))).toBe(false)
    expect(canTestPocketModel(model({ readiness: 'needs_reference' }))).toBe(false)
    expect(canTestPocketModel(null)).toBe(false)
  })

  it('builds language-specific test phrases and avoids immediate repetition', () => {
    expect(pocketTestPhrases('ru').every((phrase) => /[А-Яа-яЁё]/.test(phrase))).toBe(true)
    expect(pocketTestPhrases('en').every((phrase) => /[A-Za-z]/.test(phrase))).toBe(true)
    expect(pocketTestPhrases('ru-RU')).toBe(pocketTestPhrases('ru'))

    const ruFirst = pocketTestPhrases('ru')[0]
    expect(pickPocketTestPhrase('ru', ruFirst, () => 0)).not.toBe(ruFirst)
    expect(pickPocketTestPhrase('en', null, () => 0)).toBe(pocketTestPhrases('en')[0])
    expect(pickPocketTestPhrase('de', null, () => 0)).toBe(pocketTestPhrases('en')[0])
  })
})
