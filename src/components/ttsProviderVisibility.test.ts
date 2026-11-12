import { describe, expect, it } from 'vitest'
import {
  BUILTIN_PROVIDER_IDS,
  PIPER_VISIBILITY_ID,
  deriveLegacyVisibleIds,
  forceActiveVisible,
  isPersistedVisibility,
  normalizeVisibleIds,
  normalizeVisibilityId,
  toggleVisibleId,
} from './ttsProviderVisibility'

describe('TTS provider visibility rules', () => {
  it('keeps stable built-in provider ids in a fixed order', () => {
    expect(BUILTIN_PROVIDER_IDS).toEqual(['silero', 'openai', 'fish', 'elevenlabs', 'local-http'])
  })

  it('derives legacy visible ids as Silero + configured + active, omitting unselected Piper models', () => {
    expect(
      deriveLegacyVisibleIds({
        activeProviderId: 'openai',
        configuredIds: ['openai', 'fish'],
        piperIds: ['local-piper:amy', 'local-piper:giga'],
      }),
    ).toEqual(['silero', 'openai', 'fish'])

    expect(
      deriveLegacyVisibleIds({
        activeProviderId: 'local-piper:amy',
        configuredIds: [],
        piperIds: ['local-piper:amy', 'local-piper:giga'],
      }),
    ).toEqual(['silero', PIPER_VISIBILITY_ID])
  })

  it('includes a configured ElevenLabs provider in legacy-derived visibility', () => {
    expect(
      deriveLegacyVisibleIds({
        activeProviderId: null,
        configuredIds: ['openai', 'elevenlabs'],
        piperIds: [],
      }),
    ).toEqual(['silero', 'openai', 'elevenlabs'])
  })

  it('keeps Silero visible even without configured providers', () => {
    expect(
      deriveLegacyVisibleIds({ activeProviderId: null, configuredIds: [], piperIds: [] }),
    ).toEqual(['silero'])
  })

  it('deduplicates the active provider already present in the derived list', () => {
    expect(
      deriveLegacyVisibleIds({
        activeProviderId: 'fish',
        configuredIds: ['fish'],
        piperIds: [],
      }),
    ).toEqual(['silero', 'fish'])
  })

  it('collapses legacy Piper ids into the single piper visibility entry', () => {
    expect(normalizeVisibilityId('local-piper:amy')).toBe(PIPER_VISIBILITY_ID)
    expect(normalizeVisibilityId('local-piper:giga')).toBe(PIPER_VISIBILITY_ID)
    expect(normalizeVisibilityId('silero')).toBe('silero')
    expect(
      normalizeVisibleIds([
        'silero',
        'local-piper:amy',
        'local-piper:giga',
        'piper',
        'openai',
      ]),
    ).toEqual(['silero', 'piper', 'openai'])
  })

  it('forces the active provider id to stay visible', () => {
    expect(forceActiveVisible(['silero'], 'openai')).toEqual(['silero', 'openai'])
    expect(forceActiveVisible(['silero', 'openai'], 'openai')).toEqual(['silero', 'openai'])
    expect(forceActiveVisible(['silero'], null)).toEqual(['silero'])
    expect(forceActiveVisible(['silero', 'local-piper:amy'], 'local-piper:giga')).toEqual([
      'silero',
      'piper',
    ])
  })

  it('toggles only inactive ids on and off', () => {
    expect(toggleVisibleId(['silero'], 'openai', null)).toEqual(['silero', 'openai'])
    expect(toggleVisibleId(['silero', 'openai'], 'openai', null)).toEqual(['silero'])
    expect(toggleVisibleId(['silero'], 'silero', null)).toEqual([])
  })

  it('never toggles the active provider entry', () => {
    expect(toggleVisibleId(['silero'], 'silero', 'silero')).toEqual(['silero'])
    expect(toggleVisibleId(['silero', 'openai'], 'openai', 'openai')).toEqual(['silero', 'openai'])
    expect(toggleVisibleId(['silero'], PIPER_VISIBILITY_ID, 'local-piper:amy')).toEqual(['silero'])
  })

  it('unchecking Piper clears legacy per-model IDs', () => {
    expect(
      toggleVisibleId(['silero', 'local-piper:amy', 'local-piper:giga'], 'piper', null),
    ).toEqual(['silero'])
  })

  it('toggling Piper back on persists the single entry', () => {
    expect(toggleVisibleId(['silero'], 'piper', null)).toEqual(['silero', 'piper'])
  })

  it('recognises a persisted non-empty visibility list', () => {
    expect(isPersistedVisibility(['silero'])).toBe(true)
    expect(isPersistedVisibility([])).toBe(false)
    expect(isPersistedVisibility(undefined)).toBe(false)
    expect(isPersistedVisibility(null)).toBe(false)
  })
})
