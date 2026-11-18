import { describe, it, expect } from 'vitest'
import { presentCommandError } from '../../ipc/commandError'
import { t } from '../../i18n'
import { withLocale } from '../../test-utils/i18n'

const deviceErrorKeys = [
  'audio.error.output_format',
  'audio.error.load_devices',
  'audio.error.set_speaker_device',
  'audio.error.set_speaker_enabled',
  'audio.error.set_speaker_volume',
  'audio.error.set_virtual_mic_device',
  'audio.error.set_virtual_mic_enabled',
  'audio.error.set_virtual_mic_volume',
  'audio.error.test_speaker',
  'audio.error.test_virtual_mic',
]

describe('AudioDevicesTab error localization', () => {
  for (const key of deviceErrorKeys) {
    it(`presents the English fallback instead of a raw Russian error for ${key}`, () => {
      const raw = 'Внутренняя ошибка IPC при работе с аудиоустройством'
      withLocale('en', () => {
        const presented = presentCommandError(raw, t(key))
        expect(presented).not.toBe(raw)
        expect(presented).not.toMatch(/[А-Яа-яЁё]/)
      })
    })

    it(`presents the Russian fallback for ${key}`, () => {
      const raw = 'Внутренняя ошибка IPC при работе с аудиоустройством'
      withLocale('ru', () => {
        expect(presentCommandError(raw, t(key))).not.toBe(raw)
      })
    })
  }
})
