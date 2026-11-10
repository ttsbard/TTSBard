import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest'

const { mockInvoke } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
}))

vi.mock('../utils/debug', () => ({
  debugLog: vi.fn(),
  debugError: vi.fn(),
}))

import { useTelegramAuth } from './useTelegramAuth'
import type { TelegramStatus } from './useTelegramAuth'
import type { AppSettingsDto } from '../types/settings'
import { i18n } from '../i18n'
import { withLocale } from '../test-utils/i18n'
import ruCatalog from '../../locales/ru.json'
import enCatalog from '../../locales/en.json'

beforeAll(() => {
  i18n.global.setLocaleMessage('ru', (ruCatalog as { messages: Record<string, string> }).messages)
  i18n.global.setLocaleMessage('en', (enCatalog as { messages: Record<string, string> }).messages)
  ;(i18n.global.locale as unknown as { value: string }).value = 'ru'
})

function mockUser(): TelegramStatus {
  return {
    connected: true,
    phone: '+79991234567',
    username: 'testuser',
    first_name: 'Test',
    last_name: 'User',
  }
}

function mockSettings(): AppSettingsDto {
  return {
    storage: { data_dir: null, audio_cache_dir: null },
    tts: {
      provider: 'silero',
      provider_id: 'id1',
      providers: [],
      openai: { api_key: null, voice: 'alloy', proxy_host: null, proxy_port: null, use_proxy: false },
      local: { url: '' },
      fish: { api_key: null, voices: [], reference_id: '', format: 'wav', temperature: 0.7, sample_rate: 44100, use_proxy: false },
      elevenlabs: { api_key: null, voice_id: '', voices: [], models: [], model_id: '', output_format: 'mp3_44100_128', stability: 0.5, similarity_boost: 0.75, style: 0, use_speaker_boost: true, use_proxy: false },
      telegram: { api_id: null, proxy_mode: 'none', voices: [], current_voice_id: '', synthesis_response_timeout_ms: 10000, download_retry_delay_ms: 1000 },
      network: { proxy: { proxy_url: null }, mtproxy: { host: null, port: 443, secret: null, dc_id: null } },
    },
    webview: { enabled: false, start_on_boot: false, port: 8080, access_token: null, upnp_enabled: false, send_original_text: true },
    twitch: { mode: 'irc', enabled: false, username: '', token: '', channel: '', start_on_boot: false, send_original_text: true },
    windows: {
      global: { exclude_from_capture: false },
      main: { x: null, y: null, custom_background: false, opacity: 100, bg_color: '', custom_opacity: false, opacity_compact_only: false, compact_width: 400, compact_height: 300, compact_view: 'compact', hide_extra_window_buttons: false },
      soundpanel: { x: null, y: null, opacity: 100, bg_color: '', clickthrough: false, stay_visible: false, hide_on_blur: false, appearance_source: '' },
      playback: { x: null, y: null, opacity: 100, bg_color: '', appearance_source: '' },
    },
    audio: { output_format: 'default', speaker_device: null, speaker_enabled: true, speaker_volume: 100, virtual_mic_device: null, virtual_mic_volume: 100 },
    audio_effects: { enabled: false, pitch: 0, speed: 0, volume: 100, enhance_enabled: false, enhance_atten_db: 10, formant_preserved: true, boundary_cleanup_enabled: true },
    dsp: {
      eq: { enabled: false, low_cut_enabled: false, low_cut_hz: 80, low_cut_slope_db: 12, bands: [], high_shelf_enabled: false, high_shelf_hz: 8000, high_shelf_gain_db: 0 },
      compressor: { enabled: false, threshold_db: -20, ratio: 4, attack_ms: 5, release_ms: 50, knee_db: 6, makeup_db: 0 },
      limiter: { enabled: false, ceiling_db: -1, release_ms: 50 },
    },
    general: { hotkey_enabled: true, theme: 'dark', ui_language: 'ru', ui_font_family: 'default', ui_font_size_px: 16, show_playback_on_start: false, start_compact: false, hide_on_minimize: false },
    logging: { enabled: true, level: 'info', module_levels: {} },
    preprocessor: { enabled: false, replacements_count: 0 },
    soundpanel_bindings: [],
    editor: { quick: 'disabled', ai: false, ai_completion: false, autocomplete_enabled: true, number_conversion_enabled: true, spellcheck_enabled: false, spellcheck_source: 'online', editor_height: 200, typing_idle_timeout_ms: 800, typing_enabled: true, default_route: 'everywhere', keep_text_after_send: false, font_family: 'default', font_size_px: 16, homograph_accentor: { enabled: false, accentor_pack_id: null, load_on_start: false } },
    ocr: { enabled: false, model_id: null, capture_target: { type: 'all' } },
    ai: {
      provider: 'openai',
      openai: { api_key: null, use_proxy: false, model: 'gpt-4o-mini' },
      zai: { url: null, api_key: null, model: 'glm-4' },
      deepseek: { api_key: null, use_proxy: false, model: 'deepseek-v4-pro' },
      custom: { url: null, api_key: null, use_proxy: false, model: 'default' },
      prompt: '',
      timeout: 30000,
    },
    hotkeys: {
      main_window: { modifiers: [], key: '' },
      sound_panel: { modifiers: [], key: '' },
      playback_pause: { modifiers: [], key: '' },
      playback_stop: { modifiers: [], key: '' },
      playback_repeat: { modifiers: [], key: '' },
      playback_control_window: { modifiers: [], key: '' },
      return_previous_window: { modifiers: [], key: '' },
      toggle_minimal_mode: { modifiers: [], key: '' },
      ocr_capture: { modifiers: [], key: '' },
      editor: {
        edit_word: { modifiers: [], key: '' },
        submit_continue: { modifiers: [], key: '' },
        submit_keep_text: { modifiers: [], key: '' },
        submit_keep_focus: { modifiers: [], key: '' },
        next_spelling_error: { modifiers: [], key: '' },
        previous_spelling_error: { modifiers: [], key: '' },
        next_tab: { modifiers: [], key: '' },
        previous_tab: { modifiers: [], key: '' },
        cycle_route: { modifiers: [], key: '' },
        toggle_typing: { modifiers: [], key: '' },
        cycle_quick_mode: { modifiers: [], key: '' },
        toggle_history: { modifiers: [], key: '' },
        accent_homographs: { modifiers: [], key: '' },
        approve_next_incoming: { modifiers: [], key: '' },
        edit_next_incoming: { modifiers: [], key: '' },
      },
    },
    vtube_studio: {
      enabled: false,
      host: '127.0.0.1',
      port: 8001,
      start_on_boot: false,
      typingAction: { outputMode: 'Event', parameterName: 'TTSBardTyping', startHotkeyId: '', stopHotkeyId: '', startHotkeyName: '', stopHotkeyName: '', itemFileName: '', itemType: '' },
    },
    vrchat: { enabled: false, start_on_boot: false, host: '127.0.0.1', port: 9000 },
  }
}

const credentials = { phone: '+79991234567', api_id: '12345', api_hash: 'abcdef' }

describe('useTelegramAuth', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockInvoke.mockResolvedValue(undefined)
  })

  describe('getStatus', () => {
    it('sets state to connected when authorized', async () => {
      mockInvoke
        .mockResolvedValueOnce(true)
        .mockResolvedValueOnce(mockUser())

      const { getStatus, state, status, isConnected } = useTelegramAuth()
      const result = await getStatus()

      expect(result).toEqual(mockUser())
      expect(state.value).toBe('connected')
      expect(status.value).toEqual(mockUser())
      expect(isConnected.value).toBe(true)
    })

    it('falls back to connected when user info fails', async () => {
      mockInvoke
        .mockResolvedValueOnce(true)
        .mockRejectedValueOnce(new Error('user fetch failed'))

      const { getStatus, state, status } = useTelegramAuth()
      const result = await getStatus()

      expect(result).toEqual({ connected: true })
      expect(state.value).toBe('connected')
      expect(status.value).toEqual({ connected: true })
    })

    it('sets state to idle when not authorized', async () => {
      mockInvoke.mockResolvedValueOnce(false)

      const { getStatus, state, status, canInit } = useTelegramAuth()
      const result = await getStatus()

      expect(result).toBeNull()
      expect(state.value).toBe('idle')
      expect(status.value).toBeNull()
      expect(canInit.value).toBe(true)
    })

    it('sets state to idle on not-initialized rejection', async () => {
      mockInvoke.mockRejectedValueOnce({
        code: 'telegram.not_initialized',
        message: 'wording may change freely',
        retryable: false,
      })

      const { getStatus, state, status } = useTelegramAuth()
      const result = await getStatus()

      expect(result).toBeNull()
      expect(state.value).toBe('idle')
      expect(status.value).toBeNull()
    })

    it('does not derive idle state from a human-readable error substring', async () => {
      mockInvoke.mockRejectedValueOnce('Error: client not initialized')

      const { getStatus, state } = useTelegramAuth()
      await getStatus()

      expect(state.value).toBe('error')
    })

    it('sets state to error on unexpected error', async () => {
      mockInvoke.mockRejectedValueOnce('network error')

      const { getStatus, state, hasError } = useTelegramAuth()
      await getStatus()

      expect(state.value).toBe('error')
      expect(hasError.value).toBe(true)
    })
  })

  describe('requestCode', () => {
    it('transitions to code_required on success', async () => {
      mockInvoke
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce(undefined)

      const { requestCode, state, needsCode } = useTelegramAuth()
      const result = await requestCode(credentials)

      expect(result).toBe(true)
      expect(state.value).toBe('code_required')
      expect(needsCode.value).toBe(true)
      expect(mockInvoke).toHaveBeenCalledWith('telegram_init', expect.objectContaining({
        apiId: 12345,
        apiHash: 'abcdef',
        phone: '+79991234567',
      }))
      expect(mockInvoke).toHaveBeenCalledWith('telegram_request_code')
    })

    it('transitions to error on Error rejection', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('auth failed'))

      const { requestCode, state, hasError } = useTelegramAuth()
      const result = await requestCode(credentials)

      expect(result).toBe(false)
      expect(state.value).toBe('error')
      expect(hasError.value).toBe(true)
    })

    it('transitions to error on string rejection', async () => {
      mockInvoke.mockRejectedValueOnce('string error')

      const { requestCode, state } = useTelegramAuth()
      const result = await requestCode(credentials)

      expect(result).toBe(false)
      expect(state.value).toBe('error')
    })
  })

  describe('signIn', () => {
    it('transitions to connected on success', async () => {
      const user = mockUser()
      mockInvoke
        .mockResolvedValueOnce('Connected')
        .mockResolvedValueOnce(user)

      const { signIn, state, status, isConnected } = useTelegramAuth()
      const result = await signIn('12345')

      expect(result).toBe(true)
      expect(state.value).toBe('connected')
      expect(status.value).toEqual(user)
      expect(isConnected.value).toBe(true)
      expect(mockInvoke).toHaveBeenCalledWith('telegram_sign_in', { code: '12345' })
    })

    it('transitions to password_required', async () => {
      mockInvoke.mockResolvedValueOnce('PasswordRequired')

      const { signIn, state, needsPassword, errorMessage } = useTelegramAuth()
      const result = await signIn('12345')

      expect(result).toBe(false)
      expect(state.value).toBe('password_required')
      expect(needsPassword.value).toBe(true)
      expect(errorMessage.value).toBeNull()
    })

    it('transitions to idle on RestartRequired', async () => {
      mockInvoke.mockResolvedValueOnce('RestartRequired')

      const { signIn, state, errorMessage } = useTelegramAuth()
      const result = await signIn('12345')

      expect(result).toBe(false)
      expect(state.value).toBe('idle')
      expect(errorMessage.value).toBe('Сессия устарела. Пожалуйста, запросите код заново.')
    })

    it('transitions to error on unexpected response', async () => {
      mockInvoke.mockResolvedValueOnce('SomethingElse')

      const { signIn, state, errorMessage } = useTelegramAuth()
      const result = await signIn('12345')

      expect(result).toBe(false)
      expect(state.value).toBe('error')
      expect(errorMessage.value).toBe('Неожиданный ответ от сервера')
    })

    it('transitions to error on Error rejection', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('network error'))

      const { signIn, state } = useTelegramAuth()
      const result = await signIn('12345')

      expect(result).toBe(false)
      expect(state.value).toBe('error')
    })
  })

  describe('reset', () => {
    it('resets state to idle and clears error', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('some error'))

      const { signIn, reset, state, errorMessage } = useTelegramAuth()
      await signIn('12345')

      expect(state.value).toBe('error')
      expect(errorMessage.value).toBeDefined()

      reset()

      expect(state.value).toBe('idle')
      expect(errorMessage.value).toBeNull()
    })
  })

  describe('checkPassword', () => {
    it('transitions to connected on success', async () => {
      const user = mockUser()
      mockInvoke
        .mockResolvedValueOnce('Connected')
        .mockResolvedValueOnce(user)

      const { checkPassword, state, status } = useTelegramAuth()
      const result = await checkPassword('mypassword')

      expect(result).toBe(true)
      expect(state.value).toBe('connected')
      expect(status.value).toEqual(user)
    })

    it('transitions to error on unexpected response', async () => {
      mockInvoke.mockResolvedValueOnce('BadPassword')

      const { checkPassword, state, errorMessage } = useTelegramAuth()
      const result = await checkPassword('mypassword')

      expect(result).toBe(false)
      expect(state.value).toBe('error')
      expect(errorMessage.value).toBe('Неожиданный ответ от сервера')
    })

    it('transitions to idle on RestartRequired (explicit state)', async () => {
      mockInvoke.mockResolvedValueOnce('RestartRequired')

      const { checkPassword, state, errorMessage } = useTelegramAuth()
      const result = await checkPassword('mypassword')

      expect(result).toBe(false)
      expect(state.value).toBe('idle')
      expect(errorMessage.value).toBe('Сессия устарела. Пожалуйста, запросите код заново.')
    })

    it('stays in password_required on invalid password Error rejection', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('Неверный пароль'))

      const { checkPassword, state } = useTelegramAuth()
      const result = await checkPassword('mypassword')

      expect(result).toBe(false)
      expect(state.value).toBe('password_required')
    })

    it('stays in password_required on string rejection', async () => {
      mockInvoke.mockRejectedValueOnce('Неверный пароль')

      const { checkPassword, state } = useTelegramAuth()
      const result = await checkPassword('mypassword')

      expect(result).toBe(false)
      expect(state.value).toBe('password_required')
    })
  })

  describe('signOut', () => {
    it('clears state on success', async () => {
      mockInvoke.mockResolvedValueOnce(undefined)

      const { signOut, state, status } = useTelegramAuth()
      const result = await signOut()

      expect(result).toBe(true)
      expect(state.value).toBe('idle')
      expect(status.value).toBeNull()
    })

    it('transitions to error on failure', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('disconnect failed'))

      const { signOut, state } = useTelegramAuth()
      const result = await signOut()

      expect(result).toBe(false)
      expect(state.value).toBe('error')
    })
  })

  describe('operation guard', () => {
    it('cancelConnection invalidates in-flight requestCode', async () => {
      let resolveInit!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveInit = r }))

      const { requestCode, cancelConnection, state, loading } = useTelegramAuth()
      const promise = requestCode(credentials)

      await new Promise(r => setTimeout(r, 10))
      await cancelConnection()

      expect(state.value).toBe('idle')

      resolveInit(undefined)
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('reset invalidates in-flight requestCode', async () => {
      let resolveInit!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveInit = r }))

      const { requestCode, reset, state, loading } = useTelegramAuth()
      const promise = requestCode(credentials)

      await new Promise(r => setTimeout(r, 10))
      reset()

      expect(state.value).toBe('idle')

      resolveInit(undefined)
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('cancelConnection invalidates in-flight signIn', async () => {
      let resolveSignIn!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveSignIn = r }))

      const { signIn, cancelConnection, state, loading } = useTelegramAuth()
      const promise = signIn('12345')

      await new Promise(r => setTimeout(r, 10))
      await cancelConnection()

      expect(state.value).toBe('idle')

      resolveSignIn('Connected')
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('reset invalidates in-flight signIn', async () => {
      let resolveSignIn!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveSignIn = r }))

      const { signIn, reset, state, loading } = useTelegramAuth()
      const promise = signIn('12345')

      await new Promise(r => setTimeout(r, 10))
      reset()

      expect(state.value).toBe('idle')

      resolveSignIn('Connected')
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('cancelConnection prevents stale writes from in-flight checkPassword', async () => {
      let resolveCheck!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveCheck = r }))

      const { checkPassword, cancelConnection, state, loading } = useTelegramAuth()
      const promise = checkPassword('mypass')

      await new Promise(r => setTimeout(r, 10))
      await cancelConnection()

      expect(state.value).toBe('idle')

      resolveCheck('Connected')
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('reset invalidates in-flight checkPassword', async () => {
      let resolveCheck!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveCheck = r }))

      const { checkPassword, reset, state, loading } = useTelegramAuth()
      const promise = checkPassword('mypass')

      await new Promise(r => setTimeout(r, 10))
      reset()

      expect(state.value).toBe('idle')

      resolveCheck('Connected')
      await promise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
    })

    it('cancel guards state and errorMessage against stale Error in signIn', async () => {
      let resolveSignIn!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveSignIn = r }))

      const { signIn, cancelConnection, state, errorMessage } = useTelegramAuth()
      const promise = signIn('12345')

      await new Promise(r => setTimeout(r, 10))
      await cancelConnection()

      expect(state.value).toBe('idle')
      const savedError = errorMessage.value

      resolveSignIn(Promise.reject(new Error('stale error')))
      await promise

      expect(state.value).toBe('idle')
      expect(errorMessage.value).toBe(savedError)
    })

    it('deferred disconnect: cancelConnection sets idle/loading=false before disconnect await yields', async () => {
      let resolveInit!: (value: unknown) => void
      let resolveDisconnect!: (value: unknown) => void
      mockInvoke
        .mockImplementationOnce(() => new Promise(r => { resolveInit = r }))
        .mockImplementationOnce(() => new Promise(r => { resolveDisconnect = r }))

      const { requestCode, cancelConnection, state, loading, errorMessage, status } = useTelegramAuth()
      const reqPromise = requestCode(credentials)

      await new Promise(r => setTimeout(r, 10))
      expect(state.value).toBe('loading')
      expect(loading.value).toBe(true)

      const cancelPromise = cancelConnection()

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
      expect(errorMessage.value).toBeNull()
      expect(status.value).toBeNull()

      resolveInit(undefined)
      await reqPromise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)

      resolveDisconnect(undefined)
      await cancelPromise

      expect(state.value).toBe('idle')
      expect(loading.value).toBe(false)
      expect(errorMessage.value).toBeNull()
      expect(status.value).toBeNull()
    })
  })

  describe('init', () => {
    it('restores session and calls getStatus on success', async () => {
      const user = mockUser()
      mockInvoke
        .mockResolvedValueOnce(true)
        .mockResolvedValueOnce(true)
        .mockResolvedValueOnce(user)

      const { init, state, status } = useTelegramAuth()
      await init()

      expect(state.value).toBe('connected')
      expect(status.value).toEqual(user)
      expect(mockInvoke).toHaveBeenCalledWith('telegram_auto_restore')
    })

    it('does nothing when auto-restore returns false', async () => {
      mockInvoke.mockResolvedValueOnce(false)

      const { init, state } = useTelegramAuth()
      await init()

      expect(state.value).toBe('idle')
    })

    it('does not throw when auto-restore fails', async () => {
      mockInvoke.mockRejectedValueOnce(new Error('restore failed'))

      const { init } = useTelegramAuth()
      await expect(init()).resolves.toBeUndefined()
    })
  })

  describe('refreshLimits', () => {
    it('sets limits value on success', async () => {
      mockInvoke.mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })

      const { refreshLimits, limits, limitsLoading, limitsError } = useTelegramAuth()
      const result = await refreshLimits()

      expect(result).toEqual({ voices: '17/666', gifs: '5/50' })
      expect(limits.value).toEqual({ voices: '17/666', gifs: '5/50' })
      expect(limitsError.value).toBeNull()
      expect(limitsLoading.value).toBe(false)
    })

    it('sets limits value including reset_timestamp', async () => {
      mockInvoke.mockResolvedValueOnce({ voices: '17/666', gifs: '5/50', reset_timestamp: '07-26 14:30:00 UTC+3' })

      const { refreshLimits, limits } = useTelegramAuth()
      await refreshLimits()

      expect(limits.value).toEqual({ voices: '17/666', gifs: '5/50', reset_timestamp: '07-26 14:30:00 UTC+3' })
    })

    it('does NOT touch global errorMessage on success or failure', async () => {
      mockInvoke.mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })

      const { refreshLimits, errorMessage } = useTelegramAuth()
      await refreshLimits()

      expect(errorMessage.value).toBeNull()

      mockInvoke.mockRejectedValueOnce(new Error('limits error'))

      await refreshLimits()

      expect(errorMessage.value).toBeNull()
    })

    it('preserves previous limits value on error', async () => {
      mockInvoke
        .mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })
        .mockRejectedValueOnce('limits fetch failed')

      const { refreshLimits, limits, limitsError } = useTelegramAuth()

      await refreshLimits()
      expect(limits.value).toEqual({ voices: '17/666', gifs: '5/50' })

      await refreshLimits()
      expect(limits.value).toEqual({ voices: '17/666', gifs: '5/50' })
      expect(limitsError.value).toBe('Не удалось получить информацию о лимитах')
    })

    it('sets limitsError when backend returns null', async () => {
      mockInvoke.mockResolvedValueOnce(null)

      const { refreshLimits, limits, limitsError } = useTelegramAuth()
      const result = await refreshLimits()

      expect(result).toBeNull()
      expect(limits.value).toBeNull()
      expect(limitsError.value).toBe('Не удалось получить информацию о лимитах')
    })

    it('does not clear existing limits value when backend returns null', async () => {
      mockInvoke
        .mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })
        .mockResolvedValueOnce(null)

      const { refreshLimits, limits, limitsError } = useTelegramAuth()

      await refreshLimits()
      await refreshLimits()

      expect(limits.value).toEqual({ voices: '17/666', gifs: '5/50' })
      expect(limitsError.value).toBe('Не удалось получить информацию о лимитах')
    })

    it('preserves limits on rejection and preserves null if no prior value', async () => {
      mockInvoke.mockRejectedValueOnce('network error')

      const { refreshLimits, limits, limitsError } = useTelegramAuth()
      await refreshLimits()

      expect(limits.value).toBeNull()
      expect(limitsError.value).toBe('Не удалось получить информацию о лимитах')
    })
  })

  describe('limits stale async protection', () => {
    it('clearLimits prevents stale async completion from overwriting', async () => {
      let resolveFirst!: (value: unknown) => void
      mockInvoke.mockImplementationOnce(() => new Promise(r => { resolveFirst = r }))

      const { refreshLimits, clearLimits, limits, limitsLoading } = useTelegramAuth()
      const promise = refreshLimits()

      await new Promise(r => setTimeout(r, 10))
      clearLimits()

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)

      resolveFirst({ voices: 'old/100', gifs: 'old/10' })
      await promise

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)
    })

    it('clearLimits prevents stale error from overwriting newer result', async () => {
      let resolve!: (value: unknown) => void
      mockInvoke
        .mockImplementationOnce(() => new Promise(r => { resolve = r }))
        .mockResolvedValueOnce({ voices: 'new/100', gifs: 'new/10' })

      const { refreshLimits, clearLimits, limits } = useTelegramAuth()
      const stalePromise = refreshLimits()

      await new Promise(r => setTimeout(r, 10))
      clearLimits()

      resolve(Promise.reject(new Error('stale error')))
      await stalePromise

      await refreshLimits()

      expect(limits.value).toEqual({ voices: 'new/100', gifs: 'new/10' })
    })

    it('sequential refreshLimits overwrites with latest result', async () => {
      let resolveFirst!: (value: unknown) => void
      mockInvoke
        .mockImplementationOnce(() => new Promise(r => { resolveFirst = r }))
        .mockResolvedValueOnce({ voices: 'latest/100', gifs: 'latest/10' })

      const { refreshLimits, limits } = useTelegramAuth()
      const firstPromise = refreshLimits()

      await new Promise(r => setTimeout(r, 10))
      const secondPromise = refreshLimits()
      await secondPromise

      expect(limits.value).toEqual({ voices: 'latest/100', gifs: 'latest/10' })

      resolveFirst({ voices: 'old/100', gifs: 'old/10' })
      await firstPromise

      expect(limits.value).toEqual({ voices: 'latest/100', gifs: 'latest/10' })
    })
  })

  describe('limits clearing on terminal disconnect/reset', () => {
    it('signOut clears limits', async () => {
      mockInvoke
        .mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })
        .mockResolvedValueOnce(undefined)

      const { refreshLimits, signOut, limits, limitsLoading, limitsError } = useTelegramAuth()

      await refreshLimits()
      expect(limits.value).toBeTruthy()

      await signOut()

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)
      expect(limitsError.value).toBeNull()
    })

    it('cancelConnection clears limits', async () => {
      let resolveInit!: (value: unknown) => void
      let resolveDisconnect!: (value: unknown) => void
      mockInvoke
        .mockImplementationOnce(() => new Promise(r => { resolveInit = r }))
        .mockImplementationOnce(() => new Promise(r => { resolveDisconnect = r }))

      const { requestCode, cancelConnection, limits, limitsLoading } = useTelegramAuth()
      limits.value = { voices: '17/666', gifs: '5/50' }

      requestCode(credentials)

      await new Promise(r => setTimeout(r, 10))
      const cancelPromise = cancelConnection()

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)

      resolveInit(undefined)
      resolveDisconnect(undefined)
      await cancelPromise

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)
    })

    it('reset clears limits', async () => {
      const { refreshLimits, reset, limits, limitsLoading, limitsError } = useTelegramAuth()
      mockInvoke.mockResolvedValueOnce({ voices: '17/666', gifs: '5/50' })

      await refreshLimits()
      expect(limits.value).toBeTruthy()

      reset()

      expect(limits.value).toBeNull()
      expect(limitsLoading.value).toBe(false)
      expect(limitsError.value).toBeNull()
    })
  })

  describe('loadedSavedVoices', () => {
    it('loads voices from settings', async () => {
      const settings = mockSettings()
      settings.tts.telegram.voices = [
        { id: 'voice1', description: 'First voice' },
        { id: 'voice2' },
      ]
      mockInvoke.mockResolvedValueOnce(settings)

      const { loadSavedVoices, savedVoices } = useTelegramAuth()
      await loadSavedVoices()

      expect(savedVoices.value).toHaveLength(2)
      expect(savedVoices.value[0].id).toBe('voice1')
    })
  })

  describe('command error localization', () => {
    interface FallbackCase {
      name: string
      rawMessage: string
      invoke: (auth: ReturnType<typeof useTelegramAuth>) => Promise<unknown>
      read: (auth: ReturnType<typeof useTelegramAuth>) => string | null
      expected: Record<'ru' | 'en', string>
    }

    const cases: FallbackCase[] = [
      {
        name: 'getStatus',
        rawMessage: 'Ошибка сети при проверке статуса',
        invoke: (auth) => auth.getStatus(),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось проверить статус подключения Telegram.',
          en: 'Could not check the Telegram connection status.',
        },
      },
      {
        name: 'requestCode',
        rawMessage: 'Ошибка сети при запросе кода',
        invoke: (auth) => auth.requestCode(credentials),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось запросить код подтверждения. Попробуйте снова.',
          en: 'Could not request the confirmation code. Try again.',
        },
      },
      {
        name: 'signIn',
        rawMessage: 'Неверный код подтверждения',
        invoke: (auth) => auth.signIn('12345'),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось войти. Проверьте код и попробуйте снова.',
          en: 'Could not sign in. Check the code and try again.',
        },
      },
      {
        name: 'checkPassword',
        rawMessage: 'Неверный пароль',
        invoke: (auth) => auth.checkPassword('mypassword'),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось проверить пароль 2FA.',
          en: 'Could not verify the 2FA password.',
        },
      },
      {
        name: 'signOut',
        rawMessage: 'Ошибка отключения от сервера',
        invoke: (auth) => auth.signOut(),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось выйти из Telegram.',
          en: 'Could not sign out of Telegram.',
        },
      },
      {
        name: 'refreshLimits',
        rawMessage: 'Не удалось получить лимиты',
        invoke: (auth) => auth.refreshLimits(),
        read: (auth) => auth.limitsError.value,
        expected: {
          ru: 'Не удалось получить информацию о лимитах',
          en: 'Could not get limits information',
        },
      },
      {
        name: 'refreshVoice',
        rawMessage: 'Ошибка получения голоса',
        invoke: (auth) => auth.refreshVoice(),
        read: (auth) => auth.errorMessage.value,
        expected: {
          ru: 'Не удалось получить информацию о голосе. Проверьте подключение к боту.',
          en: 'Could not get voice information. Check your bot connection.',
        },
      },
      {
        name: 'addVoiceCode',
        rawMessage: 'Ошибка сети при добавлении голоса',
        invoke: async (auth) => {
          await expect(auth.addVoiceCode({ code: 'hamster' })).rejects.toBeTruthy()
        },
        read: (auth) => auth.voiceError.value,
        expected: {
          ru: 'Ошибка добавления голоса',
          en: 'Failed to add voice',
        },
      },
      {
        name: 'selectVoice',
        rawMessage: 'Ошибка при выборе голоса',
        invoke: async (auth) => {
          await expect(auth.selectVoice('hamster')).rejects.toBeTruthy()
        },
        read: (auth) => auth.voiceError.value,
        expected: {
          ru: 'Не удалось выбрать голос',
          en: 'Could not select voice',
        },
      },
    ]

    for (const testCase of cases) {
      for (const locale of ['ru', 'en'] as const) {
        it(`presents the ${locale} fallback instead of raw "${testCase.rawMessage}" in ${testCase.name}`, async () => {
          const auth = useTelegramAuth()
          mockInvoke.mockRejectedValueOnce(testCase.rawMessage)

          await withLocale(locale, async () => {
            await testCase.invoke(auth)
            expect(testCase.read(auth)).toBe(testCase.expected[locale])
          })
        })
      }
    }
  })
})
