import { ref, computed, getCurrentScope, onScopeDispose } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { debugLog, debugError } from '../utils/debug'
import { LocalizedError, presentCommandError, normalizeCommandError } from '../ipc/commandError'
import { t } from '../i18n'
import type { VoiceCode, AppSettingsDto } from '../types/settings'

export type TelegramAuthState = 'idle' | 'loading' | 'code_required' | 'password_required' | 'connected' | 'error'

export interface TelegramCredentials {
  phone: string
  api_id: string
  api_hash: string
}

export interface TelegramStatus {
  connected: boolean
  phone?: string
  username?: string
  first_name?: string
  last_name?: string
}

export interface TtsResult {
  success: boolean
  audio_path?: string
  duration?: number
  error?: string
}

export interface CurrentVoice {
  name: string
  id: string
}

export interface Limits {
  voices: string
  gifs: string
  reset_timestamp?: string
}

export function useTelegramAuth() {
  const state = ref<TelegramAuthState>('idle')
  const status = ref<TelegramStatus | null>(null)
  const errorMessage = ref<string | null>(null)
  const loading = ref(false)
  const currentVoice = ref<CurrentVoice | null>(null)
  const limits = ref<Limits | null>(null)
  const savedVoices = ref<VoiceCode[]>([])
  const voiceLoading = ref(false)
  const voiceError = ref<string | null>(null)
  const limitsLoading = ref(false)
  const limitsError = ref<string | null>(null)
  const limitsOpId = ref(0)
  const voiceOpId = ref(0)
  const operationId = ref(0)

  // Computed properties
  const isConnected = computed(() => state.value === 'connected')
  const isLoading = computed(() => state.value === 'loading' || loading.value)
  const needsCode = computed(() => state.value === 'code_required')
  const needsPassword = computed(() => state.value === 'password_required')
  const hasError = computed(() => state.value === 'error')
  const canInit = computed(() => state.value === 'idle' || state.value === 'error')

  // Get current Telegram connection status
  async function getStatus() {
    const opId = ++operationId.value
    try {
      loading.value = true
      const is_authorized = await invoke<boolean>('telegram_get_status')

      if (opId !== operationId.value) return status.value

      if (is_authorized) {
        // Get user info if authorized
        try {
          const user = await invoke<TelegramStatus>('telegram_get_user')
          if (opId !== operationId.value) return status.value
          status.value = user
          state.value = 'connected'
        } catch (e) {
          if (opId !== operationId.value) return status.value
          // If we can't get user info, still mark as connected
          status.value = { connected: true }
          state.value = 'connected'
        }
      } else {
        status.value = null
        state.value = 'idle'
        clearLimits()
      }

      return status.value
    } catch (error) {
      if (opId !== operationId.value) return status.value
      debugError('Failed to get Telegram status:', error)
      const commandError = normalizeCommandError(error)
      if (commandError.code === 'telegram.not_initialized') {
        status.value = null
        state.value = 'idle'
        return null
      }
      errorMessage.value = presentCommandError(error, t('tts.silero.error.status_failed'))
      state.value = 'error'
      return null
    } finally {
      if (opId === operationId.value) {
        loading.value = false
      }
    }
  }

  // Request authorization code
  async function requestCode(credentials: TelegramCredentials) {
    const opId = ++operationId.value
    try {
      loading.value = true
      errorMessage.value = null
      if (opId !== operationId.value) return false
      state.value = 'loading'

      await invoke('telegram_init', {
        apiId: Number(credentials.api_id),
        apiHash: credentials.api_hash,
        phone: credentials.phone,
      })

      if (opId !== operationId.value) return false

      await invoke('telegram_request_code')

      if (opId !== operationId.value) return false

      state.value = 'code_required'
      return true
    } catch (error) {
      if (opId !== operationId.value) return false
      debugError('Failed to request code:', error)
      errorMessage.value = presentCommandError(error, t('tts.telegram.auth.error.code_request_failed'))
      state.value = 'error'
      return false
    } finally {
      if (opId === operationId.value) {
        loading.value = false
      }
    }
  }

  // Sign in with code
  async function signIn(code: string) {
    const opId = ++operationId.value
    try {
      loading.value = true
      errorMessage.value = null
      if (opId !== operationId.value) return false
      state.value = 'loading'

      const result = await invoke<string>('telegram_sign_in', { code })

      if (opId !== operationId.value) return false

      if (result === 'Connected') {
        const user = await invoke<TelegramStatus>('telegram_get_user')
        if (opId !== operationId.value) return false
        status.value = user
        state.value = 'connected'
        errorMessage.value = null
        return true
      }

      if (result === 'PasswordRequired') {
        state.value = 'password_required'
        errorMessage.value = null
        return false
      }

      if (result === 'RestartRequired') {
        errorMessage.value = t('tts.silero.error.session_expired')
        state.value = 'idle'
        return false
      }

      errorMessage.value = t('tts.silero.error.unexpected_response')
      state.value = 'error'
      return false
    } catch (error) {
      if (opId !== operationId.value) return false
      debugError('Failed to sign in:', error)
      errorMessage.value = presentCommandError(error, t('tts.telegram.auth.error.sign_in_failed'))
      state.value = 'error'
      return false
    } finally {
      if (opId === operationId.value) {
        loading.value = false
      }
    }
  }

  async function checkPassword(password: string) {
    const opId = ++operationId.value
    try {
      loading.value = true
      errorMessage.value = null
      if (opId !== operationId.value) return false
      state.value = 'loading'

      const result = await invoke<string>('telegram_check_password', { password })

      if (opId !== operationId.value) return false

      if (result === 'Connected') {
        const user = await invoke<TelegramStatus>('telegram_get_user')
        if (opId !== operationId.value) return false
        status.value = user
        state.value = 'connected'
        errorMessage.value = null
        return true
      }

      if (result === 'RestartRequired') {
        errorMessage.value = t('tts.silero.error.session_expired')
        state.value = 'idle'
        return false
      }

      errorMessage.value = t('tts.silero.error.unexpected_response')
      state.value = 'error'
      return false
    } catch (error) {
      if (opId !== operationId.value) return false
      errorMessage.value = presentCommandError(error, t('tts.telegram.auth.error.password_check_failed'))
      state.value = 'password_required'
      return false
    } finally {
      if (opId === operationId.value) {
        loading.value = false
      }
    }
  }

  // Sign out from Telegram
  async function signOut() {
    const opId = ++operationId.value
    try {
      loading.value = true
      errorMessage.value = null

      await invoke('telegram_sign_out')

      if (opId !== operationId.value) return false

      status.value = null
      state.value = 'idle'
      clearLimits()
      return true
    } catch (error) {
      if (opId !== operationId.value) return false
      debugError('Failed to sign out:', error)
      errorMessage.value = presentCommandError(error, t('tts.silero.error.sign_out_failed'))
      state.value = 'error'
      return false
    } finally {
      if (opId === operationId.value) {
        loading.value = false
      }
    }
  }

  // Speak text using Silero TTS via Telegram
  async function speak(text: string): Promise<TtsResult> {
    try {
      debugLog('[TELEGRAM TTS] Starting synthesis for text:', text)

      const result = await invoke<TtsResult>('speak_text_silero', { text })

      debugLog('[TELEGRAM TTS] Synthesis result:', result)

      if (!result.success) {
        debugError('[TELEGRAM TTS] Synthesis failed:', result.error)
        errorMessage.value = result.error || t('tts.silero.error.unknown')
      }

      return result
    } catch (error) {
      debugError('[TELEGRAM TTS] Exception during synthesis:', error)
      errorMessage.value = presentCommandError(error, t('tts.silero.error.synthesis_failed'))
      return {
        success: false,
        error: error as string,
      }
    }
  }

  // Refresh current voice information
  async function refreshVoice(): Promise<CurrentVoice | null> {
    const opId = ++voiceOpId.value
    try {
      debugLog('[TELEGRAM VOICE] Refreshing current voice')

      const voice = await invoke<CurrentVoice | null>('telegram_get_current_voice')

      if (opId !== voiceOpId.value) return null

      debugLog('[TELEGRAM VOICE] Current voice:', voice)

      if (voice) {
        currentVoice.value = voice
        errorMessage.value = null  // Очищаем ошибку при успехе
      } else {
        // Таймаут или не удалось получить информацию
        currentVoice.value = null
        errorMessage.value = t('tts.silero.error.voice_info_failed')
      }

      return voice
    } catch (error) {
      if (opId !== voiceOpId.value) return null
      debugError('[TELEGRAM VOICE] Exception during refresh:', error)
      errorMessage.value = presentCommandError(error, t('tts.silero.error.voice_info_failed'))
      currentVoice.value = null
      return null
    }
  }

  // Refresh limits information
  async function refreshLimits(): Promise<Limits | null> {
    const opId = ++limitsOpId.value
    limitsLoading.value = true
    limitsError.value = null
    try {
      debugLog('[TELEGRAM LIMITS] Refreshing limits')

      const limitsData = await invoke<Limits | null>('telegram_get_limits')

      if (opId !== limitsOpId.value) return null

      debugLog('[TELEGRAM LIMITS] Limits:', limitsData)

      if (limitsData) {
        limits.value = limitsData
      } else {
        limitsError.value = t('tts.silero.error.limits_failed')
      }

      return limitsData
    } catch (error) {
      if (opId !== limitsOpId.value) return null
      debugError('[TELEGRAM LIMITS] Exception during refresh:', error)
      limitsError.value = presentCommandError(error, t('tts.silero.error.limits_failed'))
      return null
    } finally {
      if (opId === limitsOpId.value) {
        limitsLoading.value = false
      }
    }
  }

  function clearLimits() {
    limitsOpId.value++
    limits.value = null
    limitsLoading.value = false
    limitsError.value = null
  }

  // Initialize - check status on mount
  async function init() {
    try {
      // Try to automatically restore the session
      const restored = await invoke<boolean>('telegram_auto_restore')

      if (restored) {
        // Session restored - get user data
        await getStatus()
      } else {
        debugLog('[TELEGRAM] No session to restore')
      }
    } catch (error) {
      // Error or no session - ignore
      debugLog('[TELEGRAM] Auto-restore failed:', error)
    }
  }

  // Cancel pending auth-flow without deleting session
  async function cancelConnection() {
    operationId.value++
    state.value = 'idle'
    errorMessage.value = null
    status.value = null
    loading.value = false
    clearLimits()
    try {
      await invoke('telegram_disconnect')
    } catch (error) {
      debugError('[TELEGRAM] Failed to cancel connection:', error)
    }
  }

  // Reset to idle state
  function reset() {
    operationId.value++
    state.value = 'idle'
    errorMessage.value = null
    loading.value = false
    clearLimits()
  }

  // Load saved voices from settings
  async function loadSavedVoices() {
    try {
      debugLog('[TELEGRAM VOICES] Loading saved voices')
      const settings = await invoke<AppSettingsDto>('get_all_app_settings')
      savedVoices.value = settings.tts.telegram.voices || []
      debugLog('[TELEGRAM VOICES] Saved voices loaded:', savedVoices.value.length)
    } catch (error) {
      debugError('[TELEGRAM VOICES] Failed to load saved voices:', error)
    }
  }

  // Add voice code
  async function addVoiceCode(data: { code: string; description?: string }) {
    voiceLoading.value = true
    voiceError.value = null

    try {
      debugLog('[TELEGRAM VOICES] Adding voice code:', data.code)

      // 1. Отправить "/speaker {code}" боту
      const success = await invoke<boolean>('telegram_set_speaker', { voiceCode: data.code })

      if (success) {
        debugLog('[TELEGRAM VOICES] Speaker set successfully, getting voice info')

        // 2. Получить информацию о голосе
        const voice = await invoke<CurrentVoice>('telegram_get_current_voice')

        if (voice) {
          // 3. Добавить в сохраненные с описанием
          const voiceCode: VoiceCode = {
            id: voice.id,
            description: data.description
          }
          await invoke('telegram_add_voice_code', { voice: voiceCode })

          await loadSavedVoices()
          await refreshVoice()

          debugLog('[TELEGRAM VOICES] Voice added successfully')
        } else {
          throw new LocalizedError(t('tts.silero.error.voice_info_unavailable'))
        }
      } else {
        // Бот вернул false - значит неверный код голоса
        throw new LocalizedError(t('tts.silero.error.invalid_voice_code'))
      }
    } catch (error) {
      debugError('[TELEGRAM VOICES] Failed to add voice code:', error)
      voiceError.value = presentCommandError(error, t('tts.silero.add_voice.error'))
      throw error
    } finally {
      voiceLoading.value = false
    }
  }

  // Remove voice code
  async function removeVoiceCode(id: string) {
    try {
      debugLog('[TELEGRAM VOICES] Removing voice:', id)
      await invoke('telegram_remove_voice_code', { voiceId: id })
      await loadSavedVoices()
      debugLog('[TELEGRAM VOICES] Voice removed successfully')
    } catch (error) {
      debugError('[TELEGRAM VOICES] Failed to remove voice:', error)
      throw error
    }
  }

  // Select voice
  async function selectVoice(id: string) {
    voiceLoading.value = true
    voiceError.value = null

    try {
      debugLog('[TELEGRAM VOICES] Selecting voice:', id)
      const success = await invoke<boolean>('telegram_select_voice', { voiceId: id })

      if (success) {
        await refreshVoice()
        debugLog('[TELEGRAM VOICES] Voice selected successfully')
      } else {
        throw new LocalizedError(t('tts.silero.error.voice_select_failed'))
      }
    } catch (error) {
      debugError('[TELEGRAM VOICES] Failed to select voice:', error)
      voiceError.value = presentCommandError(error, t('tts.silero.error.voice_select_failed'))
      throw error
    } finally {
      voiceLoading.value = false
    }
  }

  // Auto-refresh voice on connect
  async function autoRefreshVoice() {
    try {
      const voice = await refreshVoice()

      if (voice) {
        // Проверить есть ли голос в списке
        const exists = savedVoices.value.some(v => v.id === voice.id)

        if (!exists) {
          // Авто-добавить новый голос
          const voiceCode: VoiceCode = { id: voice.id }
          await invoke('telegram_add_voice_code', { voice: voiceCode })
          await loadSavedVoices()
          debugLog('[TELEGRAM VOICES] Auto-added new voice:', voice)
        }
      }
    } catch (error) {
      debugError('[TELEGRAM VOICES] Auto-refresh voice failed:', error)
    }
  }

  // Unmount: инвалидируем все in-flight операции, чтобы их поздние ответы не
  // писали в уже освобождённое состояние.
  if (getCurrentScope()) {
    onScopeDispose(() => {
      operationId.value++
      voiceOpId.value++
      limitsOpId.value++
      loading.value = false
      voiceLoading.value = false
      limitsLoading.value = false
    })
  }

  return {
    // State
    state,
    status,
    errorMessage,
    loading,
    currentVoice,
    limits,
    savedVoices,
    voiceLoading,
    voiceError,
    limitsLoading,
    limitsError,

    // Computed
    isConnected,
    isLoading,
    needsCode,
    needsPassword,
    hasError,
    canInit,

    // Methods
    init,
    requestCode,
    signIn,
    checkPassword,
    signOut,
    getStatus,
    speak,
    refreshVoice,
    refreshLimits,
    clearLimits,
    cancelConnection,
    reset,
    loadSavedVoices,
    addVoiceCode,
    removeVoiceCode,
    selectVoice,
    autoRefreshVoice,
    // Generation guard
    operationId,
  }
}

// Type for the return value of useTelegramAuth
export type UseTelegramAuthReturn = ReturnType<typeof useTelegramAuth>

// Provide/inject key for sharing the Telegram auth instance
export const TELEGRAM_AUTH_KEY = Symbol('telegramAuth')
