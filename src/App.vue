<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, provide, computed, nextTick } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Music, MonitorPlay } from 'lucide-vue-next'
import Sidebar from './components/Sidebar.vue'
import InputPanel from './components/InputPanel.vue'
import TtsPanel from './components/TtsPanel.vue'
import AudioPanel from './components/AudioPanel.vue'
import PreprocessorPanel from './components/PreprocessorPanel.vue'
import WebViewPanel from './components/WebViewPanel.vue'
import TwitchPanel from './components/TwitchPanel.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import HotkeysPanel from './components/HotkeysPanel.vue'
import InterceptPanel from './components/InterceptPanel.vue'
import VTubeStudioPanel from './components/VTubeStudioPanel.vue'
import VRChatPanel from './components/VRChatPanel.vue'
import InputServerPanel from './components/InputServerPanel.vue'
import OcrPanel from './components/OcrPanel.vue'
import ErrorToasts from './components/ErrorToasts.vue'
import MinimalModeButton from './components/MinimalModeButton.vue'
import MonoModeBar from './components/MonoModeBar.vue'
import IntegrationStatusCluster from './components/titlebar/IntegrationStatusCluster.vue'
import { t } from './i18n'
import { interfaceFontCssStack, parseInterfaceFontSize, toInterfaceFontFamily, INTERFACE_FONT_SIZE_DEFAULT } from './utils/interfaceFont'
import { useTelegramAuth, TELEGRAM_AUTH_KEY } from './composables/useTelegramAuth'
import { provideAppSettings } from './composables/useAppSettings'
import { useRuAccentRuntime } from './composables/useRuAccentRuntime'
import { debugLog, debugError } from './utils/debug'
import { getInitialCompactMode, getInitialCompactView, saveCompactViewToStorage, saveStartCompactToStorage } from './composables/compactModeState'
import { createMainWindowModeController, MAIN_WINDOW_MODE_KEY } from './composables/mainWindowMode'
import { createMainWindowModeAdapter } from './composables/mainWindowModeAdapter'
import { createAsyncCleanupScope } from './utils/asyncCleanup'
import { useErrorHandler } from './composables/useErrorHandler'
import { useStartupNotifications } from './composables/useStartupNotifications'
import {
  collectSpeechQueueFailures,
  type SpeechQueueFailureKey,
} from './composables/speechQueueFailureNotifications'
import { convertOcrOneShotFailure } from './composables/ocrFailureNotifications'
import { useOcrRuntimeNotifications } from './composables/useOcrRuntimeNotifications'
import type { ModelCategory } from './composables/useModelDownloads'
import { TWITCH_DELIVERY_FAILED_EVENT, twitchDeliveryFailureLocaleKey } from './ipc/twitchDelivery'
import {
  convertUpnpForwardStatus,
  upnpFailureKey,
  UPNP_STATUS_CHANGED_EVENT,
  GET_UPNP_STATUS_COMMAND,
} from './ipc/webviewUpnp'

type Panel = 'input' | 'tts' | 'audio' | 'preprocessor' | 'webview' | 'twitch' | 'input-server' | 'vtube-studio' | 'vrchat' | 'ocr' | 'settings' | 'hotkeys' | 'intercept'

const currentPanel = ref<Panel>('input')

const inputPanelRef = ref<InstanceType<typeof InputPanel> | null>(null)
const settingsPanelRef = ref<{ openModels: (category: ModelCategory) => void } | null>(null)

const { showWarning, showError } = useErrorHandler()

// Single main-window mode owner: runtime mode, remembered compact view and
// confirmed compact geometry all live here. Children read derived computeds and
// send intents; no component owns mode-related IPC or mutable compact state.
const modeController = createMainWindowModeController({
  adapter: createMainWindowModeAdapter(),
  storage: { updateStoredView: saveCompactViewToStorage },
  boot: {
    startCompact: getInitialCompactMode(),
    compactView: getInitialCompactView(),
    compactWidth: 450,
    compactHeight: 400,
  },
  onWarning: (detail) => {
    debugError('[App] Compact exit flush failed:', detail)
    showWarning(t('shell.minimal.exit_warning'))
  },
  onViewWarning: (detail) => {
    debugError('[App] Compact view persistence failed:', detail)
    showWarning(t('general.error.save'))
  },
  onError: (detail) => {
    debugError('[App] Minimal mode transition failed:', detail)
    showError(t('shell.minimal.transition_error'))
  },
})

provide(MAIN_WINDOW_MODE_KEY, modeController)

const isMinimalMode = modeController.isMinimalMode
const isMono = modeController.isMono
const isModeBusy = modeController.busy

async function handleToggleMinimalMode() {
  const entered = await modeController.toggleMinimalMode()
  if (entered && modeController.isMinimalMode.value) {
    currentPanel.value = 'input'
  }
}

async function handleReturnCompact() {
  const ok = await modeController.setCompactView('compact')
  if (ok) {
    await nextTick()
    inputPanelRef.value?.focusEditor()
  }
}

function handleFocusEditor() {
  inputPanelRef.value?.focusEditor()
}

// Create and provide app settings context with compact snapshot hooks so the
// owner controller consumes accepted settings snapshots (revision-safe).
const needsModeReload = ref(false)
const appSettings = provideAppSettings({
  captureSnapshotToken: () => modeController.captureSnapshotRevision(),
  applySnapshot: (snapshot, token) => {
    if (!modeController.applySettingsSnapshot(snapshot, token)) needsModeReload.value = true
  },
})
// Retry rejected hydration/import snapshots once the owner is idle. The settings
// transport coalesces a reload requested while its current load is completing.
watch([needsModeReload, modeController.busy, modeController.suppressed], ([needed, busy, suppressed]) => {
  if (needed && !busy && !suppressed) {
    needsModeReload.value = false
    void appSettings.reload()
  }
})
const { dispose: disposeRuAccentRuntime } = useRuAccentRuntime()

// Show a global toast on OCR runtime startup failures even when the OCR panel
// is never opened. Registers its own status listener and boot snapshot read.
useOcrRuntimeNotifications()

useStartupNotifications(appSettings.settings)

// Create single shared instance of Telegram auth
const telegramAuth = useTelegramAuth()

// Provide it to all child components
provide(TELEGRAM_AUTH_KEY, telegramAuth)

function setPanel(panel: Panel) {
  currentPanel.value = panel
  if (panel === 'input') {
    nextTick(() => inputPanelRef.value?.focusEditor())
  }
}

async function openOcrModelManagement() {
  currentPanel.value = 'settings'
  await nextTick()
  settingsPanelRef.value?.openModels('ocr')
}

// Custom titlebar window controls
async function minimizeWindow() {
  try {
    await getCurrentWindow().minimize()
  } catch (e) {
    debugLog('[App] Failed to minimize window:', e)
  }
}

// Floating window visibility state
const soundpanelVisible = ref(false)
const playbackVisible = ref(false)
const soundpanelPending = ref(false)
const playbackPending = ref(false)
const listenerScope = createAsyncCleanupScope()
let visibilityListenerSetup: Promise<void> | null = null
let visibilityToken = 0
let speechQueueFailureKeys: ReadonlySet<SpeechQueueFailureKey> = new Set()
// Bumped by every UPnP status event so an in-flight snapshot never overwrites a
// newer transition; `upnpFailureEpisode` deduplicates repeated failures into a
// single toast per error episode.
let upnpStatusEventToken = 0
let upnpFailureEpisode = false

function applyUpnpForwardStatus(raw: unknown): void {
  const status = convertUpnpForwardStatus(raw)
  if (status.state !== 'failed') {
    upnpFailureEpisode = false
    return
  }
  if (upnpFailureEpisode) return
  upnpFailureEpisode = true
  showError(t('webview.error.upnp_forward', { reason: t(upnpFailureKey(status.code)) }))
}

function formatHotkeyDisplay(hotkey: { modifiers: string[]; key: string } | undefined): string {
  if (!hotkey || !hotkey.key) return ''
  const modMap: Record<string, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', super: 'Win' }
  const mods = hotkey.modifiers.map(m => modMap[m] ?? m)
  return mods.length > 0 ? `${mods.join('+')}+${hotkey.key}` : hotkey.key
}

const soundPanelHotkey = computed(() => appSettings.settings.value?.hotkeys?.sound_panel)
const playbackHotkey = computed(() => appSettings.settings.value?.hotkeys?.playback_control_window)

const soundPanelAction = computed(() => soundpanelVisible.value ? t('shell.soundpanel.hide') : t('shell.soundpanel.show'))
const playbackAction = computed(() => playbackVisible.value ? t('shell.playback.hide') : t('shell.playback.show'))

const soundPanelTitle = computed(() => {
  const hk = soundPanelHotkey.value?.enabled === false ? '' : formatHotkeyDisplay(soundPanelHotkey.value)
  return hk ? `${soundPanelAction.value} (${hk})` : soundPanelAction.value
})

const playbackTitle = computed(() => {
  const hk = playbackHotkey.value?.enabled === false ? '' : formatHotkeyDisplay(playbackHotkey.value)
  return hk ? `${playbackAction.value} (${hk})` : playbackAction.value
})

const hideExtraWindowButtons = computed(
  () => appSettings.settings.value?.windows?.main?.hide_extra_window_buttons ?? false
)

async function toggleSoundPanel() {
  if (soundpanelPending.value) return
  soundpanelPending.value = true
  const capturedToken = visibilityToken
  try {
    const visible = await invoke<boolean>('toggle_soundpanel_window')
    if (capturedToken === visibilityToken) {
      soundpanelVisible.value = visible
    }
  } catch (e) {
    debugError('[App] Failed to toggle sound panel:', e)
  } finally {
    soundpanelPending.value = false
  }
}

async function togglePlaybackControl() {
  if (playbackPending.value) return
  playbackPending.value = true
  const capturedToken = visibilityToken
  try {
    const visible = await invoke<boolean>('toggle_playback_control_window')
    if (capturedToken === visibilityToken) {
      playbackVisible.value = visible
    }
  } catch (e) {
    debugError('[App] Failed to toggle playback control:', e)
  } finally {
    playbackPending.value = false
  }
}

// Convert a #RRGGBB string to its RGB components (safe fallback to dark bg)
function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const match = /^#?([0-9a-fA-F]{6})$/.exec(hex?.trim() ?? '')
  if (!match) {
    return { r: 9, g: 11, b: 15 }
  }
  const value = match[1]
  return {
    r: parseInt(value.slice(0, 2), 16),
    g: parseInt(value.slice(2, 4), 16),
    b: parseInt(value.slice(4, 6), 16),
  }
}

// Normalized theme from the document attribute set in index.html before Vue
// mounts. Used only as a fallback until backend settings arrive; a light
// background is applied only when the attribute is exactly 'light'.
const initialTheme = document.documentElement.getAttribute('data-theme') === 'light' ? 'light' : 'dark'

// Dynamic background of the main window container (color + independent opacity)
const appStyle = computed(() => {
  const main = appSettings.settings.value?.windows?.main
  const theme = appSettings.settings.value?.general?.theme ?? initialTheme
  let effectiveOpacity = main?.opacity ?? 100
  if (!main?.custom_opacity) effectiveOpacity = 100
  if (main?.opacity_compact_only && !isMinimalMode.value) effectiveOpacity = 100
  const opacity = Math.min(100, Math.max(10, effectiveOpacity)) / 100
  const baseColor = main?.custom_background
    ? main.bg_color
    : theme === 'light'
      ? '#fafcff'
      : '#090b0f'
  const surfaceTint = main?.custom_background ? '20%' : '0%'
  const surfaceTintElevated = main?.custom_background ? '35%' : '0%'
  const surfaceTintHandle = main?.custom_background ? '40%' : '0%'
  const { r, g, b } = hexToRgb(baseColor)
  return {
    background: `var(--app-gradient-line), var(--app-gradient-glow), rgba(${r}, ${g}, ${b}, ${opacity})`,
    '--main-window-rgb': `${r}, ${g}, ${b}`,
    '--main-window-surface-opacity': `${opacity * 100}%`,
    '--main-window-opacity': `${opacity * 100}%`,
    '--main-window-surface-tint': surfaceTint,
    '--main-window-surface-tint-elevated': surfaceTintElevated,
    '--main-window-surface-tint-handle': surfaceTintHandle,
  }
})

// Watch for error changes
watch(() => appSettings.error.value, (newError) => {
  debugLog('[App] ⚠️ appSettings.error changed:', {
    value: newError,
    type: typeof newError,
    length: newError?.length,
    isEmpty: newError === '',
    isNull: newError === null,
    isUndefined: newError === undefined,
    truthy: !!newError
  })
  debugLog('[App] appSettings loaded:', { hasSettings: !!appSettings.settings.value, theme: appSettings.settings.value?.general?.theme, hasWindows: !!appSettings.settings.value?.windows, provider: appSettings.settings.value?.ai?.provider })
  debugLog('[App] appSettings.isLoading:', appSettings.isLoading.value)
  debugLog('[App] Will show error?', newError && newError.length > 0)
})

// Watch for settings changes
watch(() => appSettings.settings, (newSettings) => {
  debugLog('[App] ✅ appSettings.settings changed:', { hasTheme: !!newSettings?.value?.general?.theme, provider: newSettings?.value?.ai?.provider })
})

// Watch for loading state
watch(() => appSettings.isLoading, (newLoading) => {
  debugLog('[App] 🔄 appSettings.isLoading changed:', newLoading)
})

// Watch for theme changes and apply data-theme attribute
watch(() => appSettings.settings.value?.general?.theme, (newTheme, oldTheme) => {
  if (!newTheme) {
    debugLog('[App] Theme watcher: settings not loaded yet, keeping localStorage theme')
    return
  }

  debugLog('[App] Theme watcher triggered:', {
    oldTheme,
    newTheme,
    settingsLoaded: !!appSettings.settings.value,
    currentAttribute: document.documentElement.getAttribute('data-theme'),
    localStorageTheme: localStorage.getItem('app-theme')
  })
  // Save to localStorage for instant access on next app launch (prevents flash)
  localStorage.setItem('app-theme', newTheme)
  document.documentElement.setAttribute('data-theme', newTheme)
  debugLog('[App] Theme applied:', document.documentElement.getAttribute('data-theme'))
}, { immediate: true })

// Keep start_compact cached for non-native previews.
// The startup mode itself is supplied by the backend; changes apply at the next launch.
watch(
  () => appSettings.settings.value?.general?.start_compact,
  (startCompact) => {
    if (typeof startCompact !== 'boolean') return
    saveStartCompactToStorage(startCompact)
  },
  { immediate: true },
)

// Watch for interface font changes and apply the `html` rem base plus the body
// font stack. At the defaults the inline properties are removed so the
// base.css fallbacks (16 px, var(--font-sans)) stay authoritative.
watch(
  () => [
    appSettings.settings.value?.general?.ui_font_family,
    appSettings.settings.value?.general?.ui_font_size_px,
  ],
  ([rawFamily, rawSize]) => {
    const rootStyle = document.documentElement.style
    const family = toInterfaceFontFamily(rawFamily)
    const size = parseInterfaceFontSize(rawSize) ?? INTERFACE_FONT_SIZE_DEFAULT
    if (family === 'default') {
      rootStyle.removeProperty('--ui-font-family')
    } else {
      rootStyle.setProperty('--ui-font-family', interfaceFontCssStack(family))
    }
    if (size === INTERFACE_FONT_SIZE_DEFAULT) {
      rootStyle.removeProperty('--ui-font-size')
    } else {
      rootStyle.setProperty('--ui-font-size', `${size}px`)
    }
  },
  { immediate: true },
)

function keyToCode(key: string): string {
  if (key === 'SPACE') return 'Space'
  if (/^F\d+$/.test(key)) return key
  if (/^[A-Z]$/.test(key)) return `Key${key}`
  if (/^[0-9]$/.test(key)) return `Digit${key}`
  return key
}

function modifiersMatch(event: KeyboardEvent, hotkeyModifiers: string[]): boolean {
  if (hotkeyModifiers.includes('ctrl') !== event.ctrlKey) return false
  if (hotkeyModifiers.includes('shift') !== event.shiftKey) return false
  if (hotkeyModifiers.includes('alt') !== event.altKey) return false
  if (hotkeyModifiers.includes('super') !== event.metaKey) return false
  // Ensure no extra modifiers are pressed
  const expectedCount = hotkeyModifiers.length
  let actualCount = 0
  if (event.ctrlKey) actualCount++
  if (event.shiftKey) actualCount++
  if (event.altKey) actualCount++
  if (event.metaKey) actualCount++
  return expectedCount === actualCount
}

function handleReturnFocusKeydown(event: KeyboardEvent) {
  const hotkey = appSettings.settings.value?.hotkeys?.return_previous_window
  if (!hotkey || !hotkey.key || hotkey.enabled === false) return

  const expectedCode = keyToCode(hotkey.key)
  if (event.code === expectedCode && modifiersMatch(event, hotkey.modifiers)) {
    event.preventDefault()
    invoke('return_to_previous_window').catch(() => {})
  }
}

// Main-window-local hotkey (default Ctrl+M): toggle compact/normal mode.
// Handled on the frontend like return_previous_window — no global registration.
function handleToggleMinimalKeydown(event: KeyboardEvent) {
  const hotkey = appSettings.settings.value?.hotkeys?.toggle_minimal_mode
  if (!hotkey || !hotkey.key || hotkey.enabled === false) return

  const expectedCode = keyToCode(hotkey.key)
  if (event.code === expectedCode && modifiersMatch(event, hotkey.modifiers)) {
    const target = event.target as HTMLElement | null
    if (target?.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target?.tagName ?? '')) {
      return
    }
    event.preventDefault()
    void handleToggleMinimalMode()
  }
}

// Initialize Telegram session on app start
onMounted(async () => {
  debugLog('[App] 🚀 App mounted')
  debugLog('[App] Initial state:', {
    hasSettings: !!appSettings.settings,
    isLoading: appSettings.isLoading,
    error: appSettings.error,
    currentDataTheme: document.documentElement.getAttribute('data-theme'),
    localStorageTheme: localStorage.getItem('app-theme'),
    savedThemeInSettings: appSettings.settings.value?.general?.theme
  })

  document.addEventListener('keydown', handleReturnFocusKeydown)
  document.addEventListener('keydown', handleToggleMinimalKeydown)

  void listenerScope.track(
    getCurrentWindow().onFocusChanged(async ({ payload: focused }) => {
      if (focused && currentPanel.value === 'input') {
        await nextTick()
        inputPanelRef.value?.focusEditor()
      }
    }),
  ).catch((e) => {
    debugError('[App] Failed to listen for focus events:', e)
  })

  // Show a global toast when the background UPnP open attempt fails at server
  // startup, even if the failure happened before the listener was registered.
  // The snapshot is read after the listener; episode tracking deduplicates a
  // snapshot that echoes a live event into a single toast per error episode.
  const upnpListenerSetup = listenerScope.track(
    listen<unknown>(UPNP_STATUS_CHANGED_EVENT, (event) => {
      upnpStatusEventToken += 1
      applyUpnpForwardStatus(event.payload)
    }),
  ).catch((e) => {
    debugError('[App] Failed to listen for webview UPnP status events:', e)
  })

  try {
    await telegramAuth.init()
  } catch (error) {
    debugLog('[APP] Telegram auto-init failed or no session:', error)
  }

  // Listen for floating window visibility changes
  visibilityListenerSetup = listenerScope.track(
    listen<{ soundpanel_visible: boolean; playback_control_visible: boolean }>('window-visibility-changed', (event) => {
      visibilityToken++
      soundpanelVisible.value = event.payload.soundpanel_visible
      playbackVisible.value = event.payload.playback_control_visible
    }),
  ).then(() => undefined).catch(e => {
    debugError('[App] Failed to listen for visibility events:', e)
  })

  // Show a global toast when an accepted speech job transitions to failed
  void listenerScope.track(
    listen('speech-queue-changed', (event) => {
      const { notifications, currentKeys } = collectSpeechQueueFailures(
        (event as { payload: unknown }).payload,
        speechQueueFailureKeys,
      )
      speechQueueFailureKeys = currentKeys
      for (const notification of notifications) {
        showError(notification.message)
      }
    }),
  ).catch((e) => {
    debugError('[App] Failed to listen for speech queue events:', e)
  })

  // Show one unobtrusive toast for a one-shot OCR failure: an empty recognized
  // result (warning) or a fixed safe failure category (error). Unknown payloads
  // are ignored so arbitrary backend text never reaches the UI.
  void listenerScope.track(
    listen('ocr-one-shot-failed', (event) => {
      const notification = convertOcrOneShotFailure(
        (event as { payload: unknown }).payload,
      )
      if (!notification) return
      if (notification.severity === 'warning') {
        showWarning(notification.message)
      } else {
        showError(notification.message)
      }
    }),
  ).catch((e) => {
    debugError('[App] Failed to listen for OCR one-shot failure events:', e)
  })

  // Show a global toast when a Twitch delivery failure arrives, even when the
  // Twitch settings panel is closed. Only a valid `{ code, retryable }` payload
  // maps to the fixed `errors.twitch.*` message; malformed payloads are
  // debug-logged and ignored so raw backend text never reaches the UI.
  void listenerScope.track(
    listen(TWITCH_DELIVERY_FAILED_EVENT, (event) => {
      const key = twitchDeliveryFailureLocaleKey(
        (event as { payload: unknown }).payload,
      )
      if (!key) {
        debugError('[App] Ignoring malformed Twitch delivery failure payload')
        return
      }
      showError(t(key))
    }),
  ).catch((e) => {
    debugError('[App] Failed to listen for Twitch delivery failure events:', e)
  })

  // Wait for listener setup before reading snapshot to avoid the gap
  await visibilityListenerSetup

  // Load initial visibility snapshot (skip if a newer event already arrived)
  try {
    const snapshotToken = visibilityToken
    const snapshot = await invoke<{ soundpanel_visible: boolean; playback_control_visible: boolean }>('get_visibility_snapshot')
    if (snapshotToken === visibilityToken) {
      soundpanelVisible.value = snapshot.soundpanel_visible
      playbackVisible.value = snapshot.playback_control_visible
    }
  } catch (e) {
    debugError('[App] Failed to get visibility snapshot:', e)
  }

  // Wait for the UPnP listener before reading its snapshot to avoid the gap.
  await upnpListenerSetup

  // Read the UPnP snapshot (skip if a newer status event already arrived).
  try {
    const token = upnpStatusEventToken
    const snapshot = await invoke<unknown>(GET_UPNP_STATUS_COMMAND)
    if (token === upnpStatusEventToken) {
      applyUpnpForwardStatus(snapshot)
    }
  } catch (e) {
    debugError('[App] Failed to read the initial WebView UPnP status:', e)
  }
})

onUnmounted(() => {
  document.removeEventListener('keydown', handleReturnFocusKeydown)
  document.removeEventListener('keydown', handleToggleMinimalKeydown)
  listenerScope.dispose()
  disposeRuAccentRuntime()
  modeController.dispose()
})
</script>

<template>
  <div class="app-container" :style="appStyle">
    <!-- Custom title bar (frameless window) in normal/compact, MonoModeBar in mono -->
    <div v-if="!isMono" class="app-titlebar" data-tauri-drag-region>
      <div class="titlebar-logo" data-tauri-drag-region>
        <span class="titlebar-text" data-tauri-drag-region>TTSBard</span>
      </div>
      <div class="titlebar-controls">
        <IntegrationStatusCluster class="integration-gap" />
        <button
          v-if="!hideExtraWindowButtons"
          class="titlebar-btn floating-window"
          :class="{ active: soundpanelVisible }"
          @click="toggleSoundPanel"
          :title="soundPanelTitle"
          :aria-label="soundPanelAction"
          :disabled="soundpanelPending"
        >
          <Music :size="14" />
        </button>
        <button
          v-if="!hideExtraWindowButtons"
          class="titlebar-btn floating-window"
          :class="{ active: playbackVisible }"
          @click="togglePlaybackControl"
          :title="playbackTitle"
          :aria-label="playbackAction"
          :disabled="playbackPending"
        >
          <MonitorPlay :size="14" />
        </button>
        <button class="titlebar-btn minimize" @click="minimizeWindow" :title="t('shell.minimize')" :aria-label="t('shell.minimize')">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
      </div>
    </div>
    <MonoModeBar v-else @return-compact="handleReturnCompact" @focus-editor="handleFocusEditor" @minimize="minimizeWindow" />

    <!-- Show error if settings failed to load -->
    <div v-if="appSettings.error.value && appSettings.error.value.length > 0" class="error-container">
      <p>{{ t('shell.settings_load_error', { detail: appSettings.error.value }) }}</p>
      <button @click="appSettings.reload()">{{ t('shell.retry') }}</button>
    </div>

    <!-- Main app content -->
    <template v-else>
      <div class="app-content-wrapper">
        <Sidebar v-if="!isMinimalMode" :current-panel="currentPanel" @set-panel="setPanel" />

        <main class="main-content" :class="{ 'minimal-content': isMinimalMode, 'mono-content': isMono, 'input-content': currentPanel === 'input' }">
          <InputPanel ref="inputPanelRef" v-show="currentPanel === 'input'" />
          <TtsPanel v-show="currentPanel === 'tts'" />
          <AudioPanel v-show="currentPanel === 'audio'" />
          <PreprocessorPanel v-show="currentPanel === 'preprocessor'" />
          <WebViewPanel v-show="currentPanel === 'webview'" />
          <TwitchPanel v-show="currentPanel === 'twitch'" />
          <InputServerPanel v-show="currentPanel === 'input-server'" />
          <VTubeStudioPanel v-show="currentPanel === 'vtube-studio'" />
          <VRChatPanel v-show="currentPanel === 'vrchat'" />
          <OcrPanel
            v-show="currentPanel === 'ocr'"
            :active="currentPanel === 'ocr'"
            @open-models="openOcrModelManagement"
          />
          <SettingsPanel
            ref="settingsPanelRef"
            v-show="currentPanel === 'settings'"
            :active="currentPanel === 'settings'"
          />
          <HotkeysPanel v-show="currentPanel === 'hotkeys'" :active="currentPanel === 'hotkeys'" />
          <InterceptPanel v-show="currentPanel === 'intercept'" :active="currentPanel === 'intercept'" />
        </main>
      </div>

      <!-- Minimal mode toggle button (also visible in mono to restore the ordinary window) -->
      <MinimalModeButton :minimal="isMinimalMode" :busy="isModeBusy" @toggle="handleToggleMinimalMode" />

      <!-- Global error toasts -->
      <ErrorToasts />
    </template>
  </div>
</template>

<style scoped>
.app-container {
  --main-surface-elevated: color-mix(
    in srgb,
    rgb(var(--rgb-bg-elevated)) calc(100% - var(--main-window-surface-tint-elevated)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint-elevated)
  );
  --main-surface-base: color-mix(
    in srgb,
    rgb(var(--rgb-bg)) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-field: color-mix(
    in srgb,
    rgba(var(--rgb-contrast), 0.05) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-field-hover: color-mix(
    in srgb,
    rgba(var(--rgb-contrast), 0.08) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-input: color-mix(
    in srgb,
    rgba(var(--rgb-contrast), 0.16) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-select: color-mix(
    in srgb,
    color-mix(in srgb, rgb(var(--rgb-bg-elevated)), black 10%) calc(100% - var(--main-window-surface-tint-elevated)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint-elevated)
  );
  --main-surface-select-hover: color-mix(
    in srgb,
    color-mix(in srgb, rgb(var(--rgb-bg-elevated)), black 5%) calc(100% - var(--main-window-surface-tint-elevated)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint-elevated)
  );
  --main-surface-handle-top: color-mix(
    in srgb,
    rgb(var(--rgb-bg-elevated)) calc(100% - var(--main-window-surface-tint-handle)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint-handle)
  );
  --main-surface-handle-bottom: color-mix(
    in srgb,
    rgb(var(--rgb-bg)) calc(100% - var(--main-window-surface-tint-handle)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint-handle)
  );
  --color-bg: color-mix(in srgb, var(--main-surface-base) var(--main-window-surface-opacity), transparent);
  --color-bg-elevated: color-mix(in srgb, var(--main-surface-elevated) var(--main-window-surface-opacity), transparent);
  --color-bg-panel: color-mix(in srgb, color-mix(in srgb, var(--main-surface-elevated) 86%, transparent) var(--main-window-surface-opacity), transparent);
  --color-bg-panel-strong: color-mix(in srgb, color-mix(in srgb, var(--main-surface-elevated) 94%, transparent) var(--main-window-surface-opacity), transparent);
  --color-bg-field: color-mix(in srgb, var(--main-surface-field) var(--main-window-surface-opacity), transparent);
  --color-bg-field-hover: color-mix(in srgb, var(--main-surface-field-hover) var(--main-window-surface-opacity), transparent);
  --input-bg-strong: color-mix(in srgb, var(--main-surface-input) var(--main-window-surface-opacity), transparent);
  --select-bg: color-mix(in srgb, var(--main-surface-select) var(--main-window-surface-opacity), transparent);
  --select-bg-hover: color-mix(in srgb, var(--main-surface-select-hover) var(--main-window-surface-opacity), transparent);
  --sidebar-bg-top: color-mix(in srgb, color-mix(in srgb, var(--main-surface-elevated) 98%, transparent) var(--main-window-surface-opacity), transparent);
  --sidebar-bg-bottom: color-mix(in srgb, color-mix(in srgb, var(--main-surface-base) 96%, transparent) var(--main-window-surface-opacity), transparent);
  --color-bg-handle-top: color-mix(in srgb, var(--main-surface-handle-top) var(--main-window-surface-opacity), transparent);
  --color-bg-handle-bottom: color-mix(in srgb, var(--main-surface-handle-bottom) var(--main-window-surface-opacity), transparent);
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  width: 100%;
  background:
    var(--app-gradient-line),
    var(--app-gradient-glow),
    var(--app-gradient-bg);
  transition: background 0.3s ease;
}

:global([data-theme='light'] .app-container) {
  --main-surface-input: color-mix(
    in srgb,
    rgb(var(--rgb-bg-elevated)) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-field: color-mix(
    in srgb,
    rgba(var(--rgb-accent), 0.04) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --main-surface-field-hover: color-mix(
    in srgb,
    rgba(var(--rgb-accent), 0.08) calc(100% - var(--main-window-surface-tint)),
    rgb(var(--main-window-rgb)) var(--main-window-surface-tint)
  );
  --color-bg-panel: color-mix(in srgb, color-mix(in srgb, var(--main-surface-elevated) 88%, transparent) var(--main-window-surface-opacity), transparent);
}

.app-container.minimal-mode {
  transition: all 0.3s ease;
}

/* Custom frameless title bar */
.app-titlebar {
  height: 36px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 0.5rem 0 1rem;
  user-select: none;
  -webkit-user-select: none;
  color: var(--color-text-secondary);
}

.titlebar-logo {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  height: 100%;
  flex: 1;
  min-width: 0;
}

.titlebar-text {
  font-size: 0.8rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  color: var(--color-text-secondary);
}

.titlebar-controls {
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.integration-gap {
  margin-right: 0.5rem;
}

.titlebar-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.titlebar-btn:hover {
  background: var(--color-bg-field-hover);
  color: var(--color-text-primary);
}

.titlebar-btn.floating-window.active {
  color: var(--color-accent);
}

.titlebar-btn.floating-window:disabled {
  opacity: 0.5;
  cursor: default;
}

/* Wrapper holding the sidebar and main content side-by-side */
.app-content-wrapper {
  display: flex;
  flex: 1;
  overflow: hidden;
  width: 100%;
}

.main-content {
  flex: 1;
  min-width: 0;
  position: relative;
  padding: 12px 16px 3rem;
  overflow-y: auto;
  border-left: 1px solid var(--color-border);
  transition: all 0.3s ease;
}

.main-content.minimal-content {
  padding: 6px 8px 0 !important;
  overflow-y: hidden;
  scrollbar-width: none;
  transition: none;
}

.main-content.minimal-content.mono-content {
  padding: 0 4px 1px !important;
  border-left: none;
  border-top: none;
  overflow: hidden;
  height: calc(100% - 9px);
}

.main-content.minimal-content::-webkit-scrollbar {
  display: none;
}

.main-content::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: var(--grid-pattern);
  background-size: 34px 34px;
  mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 0), rgba(0, 0, 0, 0.22) 18%, rgba(0, 0, 0, 0.7));
}

.error-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  gap: 1rem;
  color: var(--toast-error-border);
}

.error-container button {
  padding: 0.5rem 1rem;
  background: var(--color-accent);
  color: var(--color-text-white);
  border: none;
  border-radius: 4px;
  cursor: pointer;
}

.error-container button:hover {
  background: var(--color-accent-strong);
}

@media (max-width: 720px) {
  .app-content-wrapper {
    flex-direction: column;
  }

  .main-content {
    padding: 12px 0.6rem 2.5rem;
    border-left: none;
    border-top: 1px solid var(--color-border);
  }
}

.main-content.input-content:not(.minimal-content) {
  padding-top: 8px;
  padding-left: 12px;
  padding-right: 12px;
}
</style>
