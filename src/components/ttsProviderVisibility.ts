import type { TtsProviderType } from '../types/settings'

/**
 * Stable concrete IDs for built-in TTS providers.
 * Piper providers get dynamic IDs (e.g. `local-piper:amy`).
 */
export const BUILTIN_PROVIDER_ID_BY_TYPE: Record<TtsProviderType, string> = {
  silero: 'silero',
  openai: 'openai',
  fish: 'fish',
  elevenlabs: 'elevenlabs',
  local: 'local-http',
}

/** Stable ordered list of all built-in provider IDs. */
export const BUILTIN_PROVIDER_IDS: readonly string[] = [
  BUILTIN_PROVIDER_ID_BY_TYPE.silero,
  BUILTIN_PROVIDER_ID_BY_TYPE.openai,
  BUILTIN_PROVIDER_ID_BY_TYPE.fish,
  BUILTIN_PROVIDER_ID_BY_TYPE.elevenlabs,
  BUILTIN_PROVIDER_ID_BY_TYPE.local,
]

/** Prefix of dynamic Piper provider IDs persisted by older versions. */
export const PIPER_PROVIDER_ID_PREFIX = 'local-piper:'

/** One visibility entry controls every discovered Piper model. */
export const PIPER_VISIBILITY_ID = 'piper'

export interface LegacyVisibilityInput {
  activeProviderId: string | null
  configuredIds: string[]
  /** Retained for callers that still pass the discovered Piper catalog. */
  piperIds: string[]
}

/**
 * Map a concrete provider ID to its visibility entry. Legacy per-model Piper
 * IDs collapse into the single `piper` entry.
 */
export function normalizeVisibilityId(id: string): string {
  return id.startsWith(PIPER_PROVIDER_ID_PREFIX) ? PIPER_VISIBILITY_ID : id
}

/** Normalize and deduplicate persisted visibility IDs while preserving order. */
export function normalizeVisibleIds(ids: readonly string[]): string[] {
  const normalized: string[] = []
  for (const id of ids) {
    if (!id) continue
    const visibilityId = normalizeVisibilityId(id)
    if (!normalized.includes(visibilityId)) normalized.push(visibilityId)
  }
  return normalized
}

function pushUnique(ids: string[], id: string): void {
  if (id && !ids.includes(id)) ids.push(id)
}

/**
 * Derive the legacy visibility set: Silero is always visible, plus every
 * configured built-in provider, plus the active provider ID. Unselected Piper
 * models are intentionally omitted; an active Piper model maps to `piper`.
 */
export function deriveLegacyVisibleIds(input: LegacyVisibilityInput): string[] {
  const ids: string[] = [BUILTIN_PROVIDER_ID_BY_TYPE.silero]
  for (const id of input.configuredIds) {
    pushUnique(ids, normalizeVisibilityId(id))
  }
  if (input.activeProviderId) {
    pushUnique(ids, normalizeVisibilityId(input.activeProviderId))
  }
  return ids
}

/**
 * Keep the active provider's visibility entry visible even when a stale saved
 * list omits it (an active Piper model forces the single `piper` entry).
 */
export function forceActiveVisible(ids: string[], activeProviderId: string | null): string[] {
  const normalized = normalizeVisibleIds(ids)
  if (!activeProviderId) return normalized
  const activeVisibilityId = normalizeVisibilityId(activeProviderId)
  if (normalized.includes(activeVisibilityId)) return normalized
  return [...normalized, activeVisibilityId]
}

/**
 * Toggle a visibility entry on/off. The active provider is never toggled, and
 * legacy `local-piper:*` entries collapse into `piper` when the list is saved.
 */
export function toggleVisibleId(ids: string[], id: string, activeProviderId: string | null): string[] {
  const normalized = normalizeVisibleIds(ids)
  const visibilityId = normalizeVisibilityId(id)
  if (activeProviderId && visibilityId === normalizeVisibilityId(activeProviderId)) {
    return normalized
  }
  if (normalized.includes(visibilityId)) {
    return normalized.filter(existing => existing !== visibilityId)
  }
  return [...normalized, visibilityId]
}

/**
 * True when a non-empty visibility list is persisted in settings.
 * An absent or empty list means legacy local derivation (nothing saved yet).
 */
export function isPersistedVisibility(visibleProviderIds: string[] | null | undefined): boolean {
  return Array.isArray(visibleProviderIds) && visibleProviderIds.length > 0
}
