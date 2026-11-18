import { describe, it, expect } from 'vitest'
import {
  parseUpdateInstallProgress,
  parseUpdateStatus,
  UPDATE_CHECK_ERROR_LOCALE_KEY,
  updateInstallErrorLocaleKey,
  type UpdateStatusDto,
} from './appUpdates'

function validStatus(overrides: Partial<UpdateStatusDto> = {}): UpdateStatusDto {
  return {
    current_version: '1.8.0',
    update_available: true,
    latest: {
      version: '1.9.0',
      name: 'TTSBard 1.9.0',
      notes: 'What is new',
      url: 'https://github.com/ttsbard/TTSBard/releases/tag/v1.9.0',
      published_at: '2026-11-16T10:00:00Z',
    },
    last_attempt_at: '2026-11-18T09:00:00Z',
    last_attempt_error: null,
    last_check_at: '2026-11-18T09:00:00Z',
    checking: false,
    startup_check_enabled: true,
    periodic_check_enabled: false,
    check_interval_days: 1,
    dismissed_version: null,
    install_available: true,
    ...overrides,
  }
}

describe('parseUpdateStatus', () => {
  it('accepts a well-formed snapshot', () => {
    const status = validStatus()
    expect(parseUpdateStatus(status)).toEqual(status)
  })

  it('accepts null optional fields, including the never-attempted state', () => {
    const status = validStatus({
      latest: null,
      last_attempt_at: null,
      last_attempt_error: null,
      last_check_at: null,
      dismissed_version: null,
    })
    expect(parseUpdateStatus(status)).toEqual(status)
  })

  it('rejects non-object payloads', () => {
    expect(parseUpdateStatus(null)).toBeNull()
    expect(parseUpdateStatus('status')).toBeNull()
    expect(parseUpdateStatus(42)).toBeNull()
  })

  it('rejects snapshots with wrong field types', () => {
    expect(parseUpdateStatus({ ...validStatus(), current_version: 1 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), update_available: 'yes' })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), checking: null })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), startup_check_enabled: 1 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), periodic_check_enabled: 'no' })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), check_interval_days: '1' })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), last_attempt_at: 1 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), last_attempt_error: 7 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), last_check_at: 1 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), dismissed_version: 1 })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), install_available: 'yes' })).toBeNull()
  })

  it('rejects snapshots with a malformed release object', () => {
    expect(parseUpdateStatus({ ...validStatus(), latest: { version: '1.9.0' } })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), latest: '1.9.0' })).toBeNull()
  })

  it('rejects unknown attempt error codes so raw text never reaches the UI', () => {
    expect(parseUpdateStatus({ ...validStatus(), last_attempt_error: 'boom: crashed' })).toBeNull()
    expect(parseUpdateStatus({ ...validStatus(), last_attempt_error: 7 })).toBeNull()
  })
})

describe('UPDATE_CHECK_ERROR_LOCALE_KEY', () => {
  // One fixed key for every failed attempt: the card deliberately shows a
  // single calm phrase instead of per-code variants.
  it('pins the single check-failure locale key', () => {
    expect(UPDATE_CHECK_ERROR_LOCALE_KEY).toBe('general.updates.errors.check_failed')
  })
})

describe('parseUpdateInstallProgress', () => {
  const validProgress = {
    stage: 'downloading',
    version: '1.9.0',
    downloaded: 2048,
    total: 102400,
    error_code: null,
  }

  it('accepts a well-formed payload and coerces optional fields', () => {
    expect(parseUpdateInstallProgress(validProgress)).toEqual(validProgress)
    expect(parseUpdateInstallProgress({ stage: 'checking', version: null, downloaded: 0, total: null, error_code: null }))
      .toEqual({ stage: 'checking', version: null, downloaded: 0, total: null, error_code: null })
    // Malformed optional fields degrade instead of rejecting the event.
    expect(parseUpdateInstallProgress({ ...validProgress, version: 1, total: 'many' }))
      .toEqual({ stage: 'downloading', version: null, downloaded: 2048, total: null, error_code: null })
  })

  it('accepts every terminal stage with a stable error code', () => {
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 'failed', error_code: 'signature' })?.stage).toBe('failed')
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 'cancelled' })?.stage).toBe('cancelled')
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 'saving' })?.stage).toBe('saving')
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 'launching' })?.stage).toBe('launching')
  })

  it('rejects unknown stages and error codes so the modal never renders unknown data', () => {
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 'exploding' })).toBeNull()
    expect(parseUpdateInstallProgress({ ...validProgress, stage: 42 })).toBeNull()
    expect(parseUpdateInstallProgress({ ...validProgress, error_code: 'boom: crashed' })).toBeNull()
    expect(parseUpdateInstallProgress(null)).toBeNull()
    expect(parseUpdateInstallProgress('progress')).toBeNull()
  })
})

describe('updateInstallErrorLocaleKey', () => {
  it('maps every stable backend code', () => {
    expect(updateInstallErrorLocaleKey('unavailable')).toBe('general.updates.install.errors.unavailable')
    expect(updateInstallErrorLocaleKey('network')).toBe('general.updates.install.errors.network')
    expect(updateInstallErrorLocaleKey('not_found')).toBe('general.updates.install.errors.not_found')
    expect(updateInstallErrorLocaleKey('signature')).toBe('general.updates.install.errors.signature')
    expect(updateInstallErrorLocaleKey('filesystem')).toBe('general.updates.install.errors.filesystem')
    expect(updateInstallErrorLocaleKey('launch')).toBe('general.updates.install.errors.launch')
    expect(updateInstallErrorLocaleKey('no_update')).toBe('general.updates.install.errors.no_update')
  })

  it('falls back to the generic key for unknown input', () => {
    expect(updateInstallErrorLocaleKey('busy')).toBe('general.updates.install.errors.generic')
    expect(updateInstallErrorLocaleKey(null)).toBe('general.updates.install.errors.generic')
    expect(updateInstallErrorLocaleKey(undefined)).toBe('general.updates.install.errors.generic')
  })
})
