<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted as vueOnUnmounted, inject, nextTick, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save } from '@tauri-apps/plugin-dialog'
import { useEditorSettings, useAppSettings, useTtsSettings, useAiSettings, useHotkeysSettings } from '../composables/useAppSettings'
import { SETTINGS_CHANGED_EVENT, type QuickEditorMode } from '../types/settings'
import { useErrorHandler } from '../composables/useErrorHandler'
import { debugLog, debugError } from '../utils/debug'
import { createAsyncCleanupScope } from '../utils/asyncCleanup'
import { MAIN_WINDOW_MODE_KEY, type MainWindowModeController } from '../composables/mainWindowMode'
import { useCompactWindowResize } from '../composables/useCompactWindowResize'
import TtsEditor from './editor/TtsEditor.vue'
import PhraseHistoryList from './PhraseHistoryList.vue'
import EditorMenu from './editor/EditorMenu.vue'
import { useEditorTabs } from '../composables/useEditorTabs'
import { registerTabFlushListener } from '../composables/tabShutdown'
import { createEditorStateCache, pruneEditorStates } from './editor/tabEditorSync'
import EditorTabs from './editor/EditorTabs.vue'
import IncomingTextsTab from './editor/IncomingTextsTab.vue'
import { useIncomingTexts } from '../composables/useIncomingTexts'
import StatusMessage from './shared/StatusMessage.vue'
import { useTypingBurst, type TypingConsumer } from '../composables/useTypingBurst'
import { acceptClear, applyAiResponse } from './inputAcceptance'
import { presentCommandError } from '../ipc/commandError'
import { submitSpeech } from '../ipc/speech'
import { deliverTwitchMessage } from '../ipc/twitchDelivery'
import { matchesEditorHotkey } from './editor/keymapArbitration'
import { incomingSurfaceHotkey, resolveIncomingTabTransition, type EditorHotkeyAction, type TabCycleDirection } from './editor/incomingHotkeys'
import { Volume2, Clock, Keyboard, Twitch, ArrowDownToLine, Undo2 } from 'lucide-vue-next'
import { enterOutcomeLabel, nextQuickMode, submitActionState, resolveKeepText } from './editor/submitAffordance'
import type { SubmitKeepIntent } from './editor/submitAffordance'
import RouteSelector from './editor/RouteSelector.vue'
import { decodeRoutePrefix, ROUTE_ORDER } from './editor/routeDecode'
import type { EditorRoute } from './editor/routeDecode'
import { effectiveRoute, applyRouteToText, routeSubmit } from './editor/routeResolution'
import { useTwitchRuntimeStatus } from '../composables/useTwitchRuntimeStatus'
import { useWebViewRuntimeStatus } from '../composables/useWebViewRuntimeStatus'
import { useRuAccentRuntime } from '../composables/useRuAccentRuntime'
import { useInputServerRuntimeStatus } from '../composables/useInputServerRuntimeStatus'
import { t } from '../i18n'

const { showError } = useErrorHandler()
const { tabs, activeId, active, create: createTab, close: closeTab, select: selectTab, next: nextTab, previous: previousTab, rename: renameTab, openIncomingEdit, init: initTabs, flushSave: flushTabsSave, lastSaveError } = useEditorTabs()

// Editor sessions (undo/redo history per tab) live only for the app run.
const editorStateCache = createEditorStateCache()
// Post-flush so it runs after the editor stashed the outgoing session.
watch(() => tabs.value.map(tab => tab.id), (ids) => {
  pruneEditorStates(editorStateCache, ids)
}, { flush: 'post' })

const {
  pendingItems: incomingPendingItems,
  externalJobs: incomingExternalJobs,
  settings: incomingSettings,
  busyIds: incomingBusyIds,
  editInFlight: incomingEditInFlight,
  loadError: incomingLoadError,
  count: incomingCount,
  approve: approveIncoming,
  edit: editIncoming,
  discard: discardIncoming,
  cancelExternalJob,
  editExternalJob,
  setAutoPlay: setIncomingAutoPlay,
  setRoute: setIncomingRoute,
} = useIncomingTexts()

const showIncomingTab = ref(false)

const { state: inputServerState } = useInputServerRuntimeStatus()

const inputServerRunning = computed(() => inputServerState.value === 'running')

const incomingTabTitle = computed(() => t('editor.incoming.title_count', { count: incomingCount.value }))

const text = computed<string>({
  get: () => active.value.text,
  set: (v) => { active.value.text = v },
})

const editorRef = ref<InstanceType<typeof TtsEditor> | null>(null)

const incomingTabRef = ref<InstanceType<typeof IncomingTextsTab> | null>(null)

async function onCreate() {
  showIncomingTab.value = false
  createTab()
  await nextTick()
  editorRef.value?.focus()
}

async function onSelect(id: string) {
  showIncomingTab.value = false
  selectTab(id)
  await nextTick()
  editorRef.value?.focus()
}

async function onClose(id: string) {
  closeTab(id)
  await nextTick()
  focusEditor()
}

async function onSelectPinned() {
  if (isMono.value) return
  showIncomingTab.value = true
  await nextTick()
  focusEditor()
}

async function onEditIncoming(id: string) {
  // Guard hotkey/programmatic paths that bypass the disabled button: never
  // start a second take while one is in flight.
  if (incomingEditInFlight.value) return
  const textToEdit = await editIncoming(id)
  if (textToEdit === null) return
  openIncomingEdit(textToEdit)
  showIncomingTab.value = false
  await nextTick()
  focusEditor()
}

async function onEditExternal(jobId: string) {
  if (incomingEditInFlight.value) return
  const textToEdit = await editExternalJob(jobId)
  if (textToEdit === null) return
  openIncomingEdit(textToEdit)
  showIncomingTab.value = false
  await nextTick()
  focusEditor()
}

const isCorrecting = ref(false)
const isCompleting = ref(false)
const isCheckingGrammar = ref(false)
const isAccenting = ref(false)
let correctionIntent = 0
let completionIntent = 0
let grammarIntent = 0
const isSpeakingInFlight = ref(false)
const showHistory = ref(false)
const saveStatusMessage = ref('')
const replacements = ref<Map<string, string>>(new Map())
const usernames = ref<Map<string, string>>(new Map())
const injectedModeController = inject<MainWindowModeController>(MAIN_WINDOW_MODE_KEY)
if (!injectedModeController) {
  throw new Error('InputPanel must be used within a component that provides MAIN_WINDOW_MODE_KEY')
}
const modeController: MainWindowModeController = injectedModeController
const isMinimalMode = modeController.isMinimalMode
const isMono = modeController.isMono
const tabsStorageKey = 'app-editor-tabs-visible'
const tabsVisible = ref((() => {
  try { return localStorage.getItem(tabsStorageKey) !== 'false' }
  catch { return true }
})())
const showTabs = computed(() => !isMono.value && (tabsVisible.value || showIncomingTab.value))

async function toggleTabs() {
  tabsVisible.value = !tabsVisible.value
  try { localStorage.setItem(tabsStorageKey, String(tabsVisible.value)) } catch { /* Storage may be unavailable. */ }
  await nextTick()
  focusEditor()
}

async function handleEnableMono() {
  const ok = await modeController.setCompactView('mono')
  if (ok && (isMinimalMode.value || await modeController.toggleMinimalMode())) {
    await nextTick()
    focusEditor()
  }
}

// In mono mode, hide phrase history and incoming surface; keep active editor tab/route.
watch(isMono, (mono) => {
  if (mono) {
    showHistory.value = false
    showIncomingTab.value = false
  }
}, { immediate: true })

const compactResize = useCompactWindowResize({ controller: modeController, showHistory, getWindow: getCurrentWindow })

const editorSettings = useEditorSettings()
const aiSettings = useAiSettings()
const ttsSettings = useTtsSettings()
const hotkeySettings = useHotkeysSettings()

const appSettingsContext = useAppSettings()
const vrchatEnabled = computed(() => appSettingsContext.settings.value?.vrchat?.enabled ?? false)

const ocrDesiredEnabled = computed(() => appSettingsContext.settings.value?.ocr.enabled ?? false)

const incomingAvailable = computed(() =>
  inputServerRunning.value || ocrDesiredEnabled.value || incomingCount.value > 0,
)

const quickEditorModeOverride = ref<QuickEditorMode | null>(null)

const quickEditorMode = computed<QuickEditorMode>(() =>
  quickEditorModeOverride.value ?? editorSettings.value?.quick ?? 'disabled',
)

watch(() => editorSettings.value?.quick, (val) => {
  if (val !== undefined) {
    quickEditorModeOverride.value = val
  }
})

const lastSubmitOutcome = ref<'none' | 'accepted' | 'error'>('none')
let submitOutcomeTimer: ReturnType<typeof setTimeout> | null = null

const submitState = computed(() => submitActionState(isSpeakingInFlight.value, lastSubmitOutcome.value))

function clearSubmitOutcomeTimer() {
  if (submitOutcomeTimer) {
    clearTimeout(submitOutcomeTimer)
    submitOutcomeTimer = null
  }
}

function scheduleOutcomeReset() {
  clearSubmitOutcomeTimer()
  submitOutcomeTimer = setTimeout(() => {
    lastSubmitOutcome.value = 'none'
    submitOutcomeTimer = null
  }, 1500)
}

function formatHotkeyDisplay(hotkey: { modifiers: string[]; key: string } | undefined): string {
  if (!hotkey || !hotkey.key) return ''
  const modMap: Record<string, string> = { ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', super: 'Win' }
  const mods = hotkey.modifiers.map(m => modMap[m] ?? m)
  return mods.length > 0 ? `${mods.join('+')}+${hotkey.key}` : hotkey.key
}

const submitContinueBinding = computed(() => formatHotkeyDisplay(hotkeySettings.value?.editor?.submit_continue))

const accentHomographsBinding = computed(() => formatHotkeyDisplay(hotkeySettings.value?.editor?.accent_homographs))

const { statusFor: ruaccentStatusFor } = useRuAccentRuntime()
const accentorPackId = computed(() => editorSettings.value?.homograph_accentor?.accentor_pack_id ?? null)
const accentorRuntimeStatus = ruaccentStatusFor(accentorPackId)
const accentorReady = computed(() => accentorRuntimeStatus.value === 'ready')

const { isConnected: twitchConnected } = useTwitchRuntimeStatus()

const { state: webviewRuntimeState } = useWebViewRuntimeStatus()
const webviewConnected = computed(() => webviewRuntimeState.value === 'running')

const defaultRouteFromSettings = computed<EditorRoute>(() => editorSettings.value?.default_route ?? 'everywhere')

const activeDecoded = computed(() => decodeRoutePrefix(text.value))

const currentRoute = computed<EditorRoute>(() =>
  effectiveRoute(activeDecoded.value, active.value.route, defaultRouteFromSettings.value),
)

const isTwitchOnly = computed(() => currentRoute.value === 'twitch_only')

function handleRouteSelect(route: EditorRoute) {
  text.value = applyRouteToText(text.value, activeDecoded.value, route)
  const tab = tabs.value.find(t => t.id === activeId.value)
  if (tab) tab.route = route
}

async function handleSaveDefault(route: EditorRoute) {
  try {
    await invoke('set_editor_default_route', { route })
  } catch (e) {
    debugError('[InputPanel] Failed to save default route:', e)
  }
}

function nextRouteFrom(current: EditorRoute, twitchConnected: boolean): EditorRoute | null {
  const allowed = ROUTE_ORDER.filter(route => route !== 'twitch_only' || twitchConnected)
  const idx = allowed.indexOf(current)
  if (idx === -1 || allowed.length <= 1) return null
  return allowed[(idx + 1) % allowed.length]
}

function cycleRoute() {
  const next = nextRouteFrom(currentRoute.value, twitchConnected.value)
  if (next === null) return
  handleRouteSelect(next)
}

const speakLabel = computed(() => {
  switch (submitState.value) {
    case 'submitting':
      return t('editor.submit.sending')
    case 'accepted':
      return t('editor.submit.accepted')
    default:
      return isTwitchOnly.value ? t('editor.submit.send') : t('editor.submit.speak')
  }
})

const speakTitle = computed(() => {
  if (submitState.value === 'submitting') return t('editor.submit.sending_enter')
  // Twitch-only: the button label stays short; the hover tooltip carries the
  // full semantics of the route.
  if (isTwitchOnly.value) return t('editor.submit.twitch_title')
  return t('editor.submit.speak_enter')
})

// Quick-editor mode indicator next to the speak button: reflects what happens
// to the window after Enter. Hidden entirely for the disabled mode.
const quickModeIndicator = computed(() => {
  switch (quickEditorMode.value) {
    case 'collapse':
      return {
        icon: ArrowDownToLine,
        title: t('editor.quick.collapse_hint'),
      }
    case 'return_focus':
      return {
        icon: Undo2,
        title: t('editor.quick.return_focus_hint'),
      }
    default:
      return null
  }
})

const speakAriaLabel = computed(() => {
  const outcome = enterOutcomeLabel(quickEditorMode.value)
  const binding = submitContinueBinding.value
  const second = binding ? `, ${binding} — ${t('editor.submit.continue_binding')}` : ''
  const action = isTwitchOnly.value ? t('editor.submit.twitch_action') : t('editor.submit.speak_action')
  return `${action}. ${t('editor.submit.enter_outcome', { outcome })}${second}`
})

const aiEditorEnabled = computed(() => editorSettings.value?.ai ?? false)

const isProviderConfigured = computed(() => {
  const provider = aiSettings.value?.provider
  const hasKey = provider === 'openai'
    ? !!aiSettings.value?.openai?.api_key
    : provider === 'zai'
      ? !!aiSettings.value?.zai?.api_key
      : provider === 'deepseek'
        ? !!aiSettings.value?.deepseek?.api_key
        : provider === 'custom'
          ? !!aiSettings.value?.custom?.api_key && !!aiSettings.value?.custom?.url
          : false

  debugLog('[InputPanel] AI provider check:', {
    provider,
    hasOpenaiKey: !!aiSettings.value?.openai?.api_key,
    hasZaiKey: !!aiSettings.value?.zai?.api_key,
    hasDeepSeekKey: !!aiSettings.value?.deepseek?.api_key,
    hasCustomKey: !!aiSettings.value?.custom?.api_key && !!aiSettings.value?.custom?.url,
    isProviderConfigured: hasKey
  })

  return hasKey
})

const isAiButtonEnabled = computed(() => {
  return isProviderConfigured.value
})

const listenerScope = createAsyncCleanupScope()

const typingBurst = useTypingBurst(
  () => editorSettings.value?.typing_idle_timeout_ms ?? 800,
  [
    {
      setTyping(active: boolean) {
        return invoke('set_vtube_studio_typing', { typing: active }).then(() => {}).catch((e) => {
          debugError('[InputPanel] VTS typing failed:', e)
        })
      },
    } satisfies TypingConsumer,
    {
      setTyping(active: boolean) {
        return invoke('set_webview_typing', { typing: active }).then(() => {}).catch((e) => {
          debugError('[InputPanel] WebView typing failed:', e)
        })
      },
    } satisfies TypingConsumer,
    {
      setTyping(active: boolean) {
        return invoke('set_vrchat_typing', { typing: active }).then(() => {}).catch((e) => {
          debugError('[InputPanel] VRChat typing failed:', e)
        })
      },
    } satisfies TypingConsumer,
  ],
)

const typingEnabledOverride = ref<boolean | null>(null)

const typingEnabled = computed(() => typingEnabledOverride.value ?? editorSettings.value?.typing_enabled ?? true)

const typingToggleTitle = computed(() =>
  t('editor.typing.toggle_title', { state: typingEnabled.value ? t('editor.typing.on') : t('editor.typing.off') }),
)

watch(() => editorSettings.value?.typing_enabled, (val) => {
  if (val !== undefined) {
    typingEnabledOverride.value = val
  }
})

function onUserEdit() {
  if (typingEnabled.value) {
    typingBurst.edit()
  }
}

function toggleTypingEnabled() {
  const prev = typingEnabled.value
  const next = !prev
  typingEnabledOverride.value = next
  if (!next) {
    typingBurst.stop()
  }
  invoke('set_editor_typing_enabled', { enabled: next }).catch((e) => {
    typingEnabledOverride.value = prev
    console.error('[InputPanel] Failed to save typing enabled:', e)
  })
}

async function cycleQuickMode() {
  const prev = quickEditorMode.value
  const next = nextQuickMode(prev)
  if (next === prev) return
  quickEditorModeOverride.value = next
  try {
    await invoke('set_editor_quick', { value: next })
  } catch (e) {
    quickEditorModeOverride.value = prev
    console.error('[InputPanel] Failed to save quick editor mode:', e)
  }
}

async function reloadPreprocessorData() {
  try {
    debugLog('[InputPanel] Reloading preprocessor data...')
    const data = await invoke<{
      replacements: Record<string, string>
      usernames: Record<string, string>
    }>('load_preprocessor_data')

    replacements.value = new Map(Object.entries(data.replacements))
    usernames.value = new Map(Object.entries(data.usernames))
    debugLog('[InputPanel] Reloaded replacements:', replacements.value.size, 'entries')
    debugLog('[InputPanel] Reloaded usernames:', usernames.value.size, 'entries')
  } catch (e) {
    debugError('[InputPanel] Failed to reload preprocessor data:', e)
  }
}

function onPreprocessorChanged() {
  debugLog('[InputPanel] Preprocessor data changed event received')
  reloadPreprocessorData()
}

onMounted(async () => {
  await initTabs()

  registerTabFlushListener(listenerScope, { flushSave: flushTabsSave, lastSaveError })

  await listenerScope.track(
    listen(SETTINGS_CHANGED_EVENT, async () => {
      debugLog('[InputPanel] Settings changed event received')
    }),
  )

  window.addEventListener('preprocessor-data-changed', onPreprocessorChanged)

  await reloadPreprocessorData()

  const currentWindow = getCurrentWindow()
  const closeHandler = async () => {
    await flushTabsSave()
  }
  await listenerScope.track(currentWindow.onCloseRequested(closeHandler))

  await compactResize.init()
})

vueOnUnmounted(async () => {
  compactResize.dispose()
  listenerScope.dispose()
  clearSubmitOutcomeTimer()
  window.removeEventListener('preprocessor-data-changed', onPreprocessorChanged)
  typingBurst.dispose()
  await flushTabsSave()
})

async function hideMainWindow() {
  try {
    await invoke('hide_main_window')
  } catch (e) {
    debugError('[InputPanel] Failed to hide window:', e)
  }
}

async function recordHistory(textToRecord: string) {
  try {
    await invoke('record_history', { text: textToRecord })
  } catch {
    // silently fail
  }
}

async function correctText() {
  if (!text.value.trim()) return
  const senderTabId = activeId.value
  const sourceText = text.value
  const token = ++correctionIntent
  isCorrecting.value = true
  try {
    const corrected = await invoke<string>('correct_text', { text: sourceText })
    tabs.value = applyAiResponse(tabs.value, senderTabId, sourceText, activeId.value, corrected)
  } catch (e) {
    debugError('[InputPanel] Correction failed:', e)
  } finally {
    if (token === correctionIntent) isCorrecting.value = false
  }
}

async function completeText() {
  if (!text.value.trim()) return
  const senderTabId = activeId.value
  const sourceText = text.value
  const token = ++completionIntent
  isCompleting.value = true
  try {
    const addition = await invoke<string>('get_ai_completion', { context: sourceText })
    if (!addition) return
    const composed = `${sourceText} ${addition}`.trim()
    tabs.value = applyAiResponse(tabs.value, senderTabId, sourceText, activeId.value, composed)
  } catch (e) {
    debugError('[InputPanel] AI completion failed:', e)
    showError(t('editor.error.ai_complete'))
  } finally {
    if (token === completionIntent) isCompleting.value = false
  }
}

async function checkGrammar() {
  if (!text.value.trim()) return
  const senderTabId = activeId.value
  const sourceText = text.value
  const token = ++grammarIntent
  isCheckingGrammar.value = true
  try {
    const corrected = await invoke<string>('ai_check_grammar', { text: sourceText })
    tabs.value = applyAiResponse(tabs.value, senderTabId, sourceText, activeId.value, corrected)
    debugLog('[InputPanel] Grammar check done')
  } catch (e) {
    debugError('[InputPanel] Grammar check failed:', e)
    showError(t('editor.error.ai_grammar'))
  } finally {
    if (token === grammarIntent) isCheckingGrammar.value = false
  }
}

async function previewHomographs() {
  if (!text.value.trim()) return
  if (isAccenting.value) return
  if (!accentorReady.value) return
  const senderTabId = activeId.value
  const sourceText = text.value
  isAccenting.value = true
  try {
    const marked = await invoke<string>('preview_contextual_ruaccent', { text: sourceText })
    tabs.value = applyAiResponse(tabs.value, senderTabId, sourceText, activeId.value, marked)
    await nextTick()
    focusEditor()
  } catch (e) {
    debugError('[InputPanel] Homograph preview failed:', e)
    showError(presentCommandError(e, t('editor.error.accent_preview')))
  } finally {
    isAccenting.value = false
  }
}

function getProviderExtension(): string {
  const provider = ttsSettings.value?.provider
  if (provider === 'fish') {
    return ttsSettings.value?.fish?.format || 'mp3'
  }
  if (provider === 'local') {
    return 'wav'
  }
  return 'mp3'
}

async function saveAudio() {
  const currentText = text.value
  if (!currentText.trim()) return

  const ext = getProviderExtension()
  try {
    const filePath = await save({
      defaultPath: `tts_export.${ext}`,
      filters: [{ name: t('editor.save_audio.filter'), extensions: [ext] }],
    })
    if (!filePath) return

    await invoke('speak_text_raw_export', { text: currentText, path: filePath })
    saveStatusMessage.value = t('editor.save_audio.saved')
  } catch (e) {
    debugError('[InputPanel] Save audio failed:', e)
    showError(presentCommandError(e, t('editor.error.save_audio')))
  }
}

watch(isMinimalMode, (minimal) => {
  if (minimal && showHistory.value) {
    showHistory.value = false
  }
})

watch(() => text.value, () => {
  lastSubmitOutcome.value = 'none'
})

watch(activeId, () => {
  lastSubmitOutcome.value = 'none'
})

watch(incomingAvailable, (available) => {
  if (!available && showIncomingTab.value) {
    showIncomingTab.value = false
  }
})

async function handleSubmit(intent: SubmitKeepIntent) {
  const currentText = text.value
  const senderTabId = activeId.value
  const mode = editorSettings.value?.quick ?? 'disabled'

  if (!currentText.trim()) return
  if (isSpeakingInFlight.value) return

  clearSubmitOutcomeTimer()
  typingBurst.stop()
  isSpeakingInFlight.value = true

  const decision = resolveKeepText(editorSettings.value?.keep_text_after_send ?? false, intent)

  const submittedDecoded = decodeRoutePrefix(currentText)
  const tabRouteAtSubmit = active.value.route
  const submitRouting = routeSubmit(submittedDecoded, tabRouteAtSubmit, defaultRouteFromSettings.value)

  try {
    if (submitRouting.twitchOnly) {
      await deliverTwitchMessage(submitRouting.outgoingText)
    } else {
      await submitSpeech(submitRouting.outgoingText)
    }
    await recordHistory(submittedDecoded.text)

    if (!decision.keepText) {
      tabs.value = acceptClear(tabs.value, senderTabId, currentText)
    }

    if (decision.applyQuickPolicy) {
      if (mode === 'collapse') {
        await hideMainWindow()
      } else if (mode === 'return_focus') {
        try {
          await invoke('return_to_previous_window')
        } catch (e) {
          debugError('[InputPanel] Failed to return focus:', e)
        }
      }
    } else {
      await nextTick()
      focusEditor()
    }

    await nextTick()
    const senderTab = tabs.value.find(t => t.id === senderTabId)
    if (activeId.value === senderTabId) {
      if (senderTab?.text === '') {
        lastSubmitOutcome.value = 'accepted'
        scheduleOutcomeReset()
      }
      if (senderTab && senderTab.route === tabRouteAtSubmit) {
        senderTab.route = undefined
      }
    }
  } catch (e) {
    debugError('[InputPanel] Failed to speak:', e)
    showError(presentCommandError(e, t('editor.error.submit')))
    lastSubmitOutcome.value = 'error'
  } finally {
    isSpeakingInFlight.value = false
  }
}

async function handleEnter() {
  await handleSubmit('quick')
}

async function handleSubmitContinue() {
  await handleSubmit('continue')
}

async function handleEsc() {
  const mode = editorSettings.value?.quick ?? 'disabled'

  typingBurst.stop()

  if (mode === 'collapse') {
    hideMainWindow()
  } else if (mode === 'return_focus') {
    try {
      await invoke('return_to_previous_window')
    } catch (e) {
      debugError('[InputPanel] Failed to return focus:', e)
    }
  }
}

function appendPhrase(newText: string) {
  const currentText = text.value
  if (!currentText.trim()) {
    text.value = newText
    return
  }
  const sep = currentText.endsWith(' ') ? '' : ' '
  text.value = currentText + sep + newText
}

function replacePhrase(newText: string) {
  const currentText = text.value
  if (currentText.trim() && currentText !== newText) {
    if (!confirm(t('history.replace_confirm'))) return
  }
  text.value = newText
}

const replacementsRecord = computed(() => {
  const obj: Record<string, string> = {}
  replacements.value.forEach((v, k) => { obj[k] = v })
  return obj
})

const usernamesRecord = computed(() => {
  const obj: Record<string, string> = {}
  usernames.value.forEach((v, k) => { obj[k] = v })
  return obj
})

// Full-mode editor height resize state. The height separator drives only the
// editor height setting (200..1200 CSS px), persisted via set_editor_height.
const editorHeight = ref(editorSettings.value?.editor_height ?? 340)
const isResizing = ref(false)
const resizeStartY = ref(0)
const resizeStartHeight = ref(0)

watch(() => editorSettings.value?.editor_height, (newVal) => {
  if (newVal !== undefined && newVal !== editorHeight.value && !isResizing.value) {
    editorHeight.value = newVal
  }
}, { immediate: true })

function onResizePointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  isResizing.value = true
  resizeStartY.value = e.clientY
  resizeStartHeight.value = editorHeight.value
  ;(e.target as HTMLElement)?.setPointerCapture?.(e.pointerId)
}

function onResizePointerMove(e: PointerEvent) {
  if (!isResizing.value) return
  const dy = e.clientY - resizeStartY.value
  editorHeight.value = Math.max(200, Math.min(1200, resizeStartHeight.value + dy))
}

function onResizePointerUp(_e: PointerEvent) {
  if (!isResizing.value) return
  isResizing.value = false
  const heightToSave = editorHeight.value
  invoke('set_editor_height', { height: heightToSave }).catch(() => {})
}

function onResizePointerCancel(_e: PointerEvent) {
  isResizing.value = false
}

function onResizeLostPointerCapture(_e: PointerEvent) {
  isResizing.value = false
}

const editorHeightPx = computed(() => `${editorHeight.value}px`)

function toggleHistory() {
  if (isMono.value) return
  showHistory.value = !showHistory.value
}

async function focusEditor() {
  if (showIncomingTab.value) {
    incomingTabRef.value?.focus()
    return
  }
  editorRef.value?.focus()
}

function cycleTabs(direction: TabCycleDirection) {
  if (isMono.value) {
    showIncomingTab.value = false
    if (direction === 'next') nextTab()
    else previousTab()
    return
  }
  const transition = resolveIncomingTabTransition(
    tabs.value.map(t => t.id),
    activeId.value,
    direction,
    incomingAvailable.value,
    showIncomingTab.value,
  )
  if (transition?.kind === 'incoming') {
    showIncomingTab.value = true
  } else if (transition?.kind === 'tab') {
    showIncomingTab.value = false
    selectTab(transition.id)
  } else {
    showIncomingTab.value = false
    if (direction === 'next') nextTab()
    else previousTab()
  }
}

async function handleEditorScopeKeydown(event: KeyboardEvent) {
  if (event.target instanceof HTMLInputElement) return
  const bindings = hotkeySettings.value?.editor
  if (!bindings) return

  if (showIncomingTab.value) {
    const matched: EditorHotkeyAction | null = matchesEditorHotkey(bindings.next_tab, event)
      ? 'next_tab'
      : matchesEditorHotkey(bindings.previous_tab, event)
        ? 'previous_tab'
        : matchesEditorHotkey(bindings.approve_next_incoming, event)
          ? 'approve_next_incoming'
          : matchesEditorHotkey(bindings.edit_next_incoming, event)
            ? 'edit_next_incoming'
            : null
    const firstPending = incomingPendingItems.value[0]
    if (matched === 'approve_next_incoming' && firstPending) {
      event.preventDefault()
      event.stopPropagation()
      void approveIncoming(firstPending.id)
    } else if (matched === 'edit_next_incoming' && firstPending) {
      event.preventDefault()
      event.stopPropagation()
      await onEditIncoming(firstPending.id)
    } else if (matched && incomingSurfaceHotkey(matched) === 'cycle_tabs') {
      event.preventDefault()
      event.stopPropagation()
      cycleTabs(matched === 'next_tab' ? 'next' : 'previous')
      await nextTick()
      focusEditor()
    }
    return
  }

  if (matchesEditorHotkey(bindings.submit_keep_focus, event)) {
    event.preventDefault()
    event.stopPropagation()
    await handleSubmit('keep_focus')
    return
  }
  if (matchesEditorHotkey(bindings.submit_keep_text, event)) {
    event.preventDefault()
    event.stopPropagation()
    await handleSubmit('keep_text')
    return
  }

  const direction: TabCycleDirection | null = matchesEditorHotkey(bindings.next_tab, event)
    ? 'next'
    : matchesEditorHotkey(bindings.previous_tab, event)
      ? 'previous'
      : null
  if (direction) {
    event.preventDefault()
    event.stopPropagation()
    cycleTabs(direction)
    await nextTick()
    focusEditor()
    return
  }

  if (matchesEditorHotkey(bindings.approve_next_incoming, event)) {
    const firstPending = incomingPendingItems.value[0]
    if (!firstPending) return
    event.preventDefault()
    event.stopPropagation()
    void approveIncoming(firstPending.id)
    return
  }
  if (matchesEditorHotkey(bindings.edit_next_incoming, event)) {
    const firstPending = incomingPendingItems.value[0]
    if (!firstPending) return
    event.preventDefault()
    event.stopPropagation()
    await onEditIncoming(firstPending.id)
    return
  }

  if (matchesEditorHotkey(bindings.accent_homographs, event)) {
    event.preventDefault()
    event.stopPropagation()
    await previewHomographs()
    return
  }

  if (matchesEditorHotkey(bindings.cycle_route, event)) {
    event.preventDefault()
    event.stopPropagation()
    cycleRoute()
    return
  }
  if (matchesEditorHotkey(bindings.toggle_typing, event)) {
    event.preventDefault()
    event.stopPropagation()
    toggleTypingEnabled()
    return
  }
  if (matchesEditorHotkey(bindings.cycle_quick_mode, event)) {
    event.preventDefault()
    event.stopPropagation()
    await cycleQuickMode()
    return
  }
  if (matchesEditorHotkey(bindings.toggle_history, event)) {
    event.preventDefault()
    event.stopPropagation()
    toggleHistory()
  }
}

defineExpose({ focusEditor })
</script>

<template>
  <div class="input-panel" :class="{ 'minimal-panel': isMinimalMode, 'mono-panel': isMono, 'history-open': showHistory }">
    <StatusMessage
      :message="saveStatusMessage"
      type="success"
      @dismiss="saveStatusMessage = ''"
    />
    <div class="input-group">
      <div class="textarea-wrapper" :class="{ 'minimal-wrapper': isMinimalMode, 'mono-wrapper': isMono, 'incoming-wrapper': showIncomingTab, 'tabs-hidden': !showTabs }" @keydown.capture="handleEditorScopeKeydown">
        <EditorTabs
          v-if="showTabs"
          :tabs="tabs"
          :active-id="activeId"
          :pinned-title="incomingAvailable ? incomingTabTitle : undefined"
          :pinned-active="incomingAvailable && showIncomingTab"
          @create="onCreate"
          @close="onClose"
          @select="onSelect"
          @rename="renameTab"
          @select-pinned="onSelectPinned"
        />
        <IncomingTextsTab
          ref="incomingTabRef"
          v-if="showIncomingTab"
          :pending-items="incomingPendingItems"
          :external-jobs="incomingExternalJobs"
          :auto-play="incomingSettings.auto_play"
          :route="incomingSettings.route"
          :twitch-connected="twitchConnected"
          :webview-connected="webviewConnected"
          :vrchat-enabled="vrchatEnabled"
          :busy-ids="incomingBusyIds"
          :edit-disabled="incomingEditInFlight"
          :compact="isMinimalMode"
          :load-error="incomingLoadError"
          @approve="approveIncoming"
          @discard="discardIncoming"
          @edit="onEditIncoming"
          @edit-external="onEditExternal"
          @cancel-external="cancelExternalJob"
          @toggle-auto-play="setIncomingAutoPlay"
          @route-change="setIncomingRoute"
        />
        <template v-else>
          <TtsEditor
            ref="editorRef"
            v-model="text"
            :editor-key="active.id"
            :state-cache="editorStateCache"
            :replacements="replacementsRecord"
            :usernames="usernamesRecord"
            :editor-height-px="editorHeightPx"
            @user-edit="onUserEdit"
            @enter="handleEnter"
            @submit-continue="handleSubmitContinue"
            @esc="handleEsc"
          />
          <div
            v-if="!isMinimalMode"
            class="editor-resize-handle"
            role="separator"
            aria-orientation="horizontal"
            :aria-label="t('editor.resize.editor_height')"
            :title="t('editor.resize.editor_height')"
            @pointerdown="onResizePointerDown"
            @pointermove="onResizePointerMove"
            @pointerup="onResizePointerUp"
            @pointercancel="onResizePointerCancel"
            @lostpointercapture="onResizeLostPointerCapture"
          />
        </template>
      </div>

      <div v-if="!showIncomingTab && !isMono" class="editor-action-bar" :class="{ 'compact-action-bar': isMinimalMode }">
        <EditorMenu
          :is-ai-enabled="isAiButtonEnabled"
          :has-text="!!text.trim()"
          :compact="isMinimalMode"
          :accenting="isAccenting"
          :accent-ready="accentorReady"
          :accent-homographs-binding="accentHomographsBinding"
          :tabs-visible="tabsVisible"
          :mode-busy="modeController.busy.value"
          @toggle-tabs="toggleTabs"
          @enable-mono="handleEnableMono"
          @correct="correctText"
          @complete="completeText"
          @grammar="checkGrammar"
          @accent-homographs="previewHomographs"
          @save-audio="saveAudio"
        />
        <button
          class="action-btn history-btn"
          :class="{ active: showHistory }"
          @click="toggleHistory"
          :title="t('history.title')"
          :aria-label="showHistory ? t('history.toggle_hide') : t('history.toggle_show')"
        >
          <Clock v-if="isMinimalMode" :size="14" />
          <template v-else>{{ t('history.title') }}</template>
        </button>
        <button
          v-if="!isMinimalMode"
          class="action-btn ai-btn"
          :class="{ loading: isCorrecting || isCompleting || isCheckingGrammar }"
          :disabled="isCorrecting || isCompleting || isCheckingGrammar || !text.trim() || !isAiButtonEnabled"
          @click="correctText"
          :title="t('editor.ai_correct.title')"
          :aria-label="t('editor.ai_correct.aria')"
        >
          AI
        </button>
        <RouteSelector
          :route="currentRoute"
          :default-route="defaultRouteFromSettings"
          :twitch-connected="twitchConnected"
          :compact="isMinimalMode"
          @select="handleRouteSelect"
          @set-default="handleSaveDefault"
        />
        <button
          type="button"
          class="action-btn typing-toggle-btn"
          :class="{ active: typingEnabled }"
          :aria-pressed="typingEnabled"
          :aria-label="typingToggleTitle"
          :title="typingToggleTitle"
          @click="toggleTypingEnabled"
        >
          <Keyboard :size="14" />
        </button>
        <div class="action-bar-spacer" />
        <span
          v-if="quickModeIndicator"
          class="quick-mode-indicator"
          role="img"
          :title="quickModeIndicator.title"
          :aria-label="quickModeIndicator.title"
        >
          <component :is="quickModeIndicator.icon" :size="13" />
        </span>
        <button
          class="action-btn speak-btn"
          :class="{ 'icon-only': isMinimalMode }"
          :disabled="!text.trim() || submitState === 'submitting'"
          :aria-busy="submitState === 'submitting'"
          :title="speakTitle"
          :aria-label="speakAriaLabel"
          @click="handleEnter"
        >
          <Twitch v-if="isTwitchOnly" :size="14" />
          <Volume2 v-else :size="14" />
          <span v-if="!isMinimalMode">{{ speakLabel }}</span>
        </button>
      </div>

      <PhraseHistoryList
        v-if="!showIncomingTab && !isMono"
        v-model:expanded="showHistory"
        :hide-toggle="true"
        @select="appendPhrase"
        @append="appendPhrase"
        @replace="replacePhrase"
      />
      <div v-if="aiEditorEnabled && !showIncomingTab && !isMono" class="ai-editor-hint">
        AI
      </div>
    </div>
  </div>
</template>

<style scoped>
.input-panel {
  position: relative;
  z-index: 1;
  max-width: 1120px;
  margin: 0;
  padding: 0 0 2rem;
  transition: all 0.3s ease;
}

.input-panel.minimal-panel {
  padding: 0 !important;
  max-width: none !important;
  height: 100%;
  display: flex;
  flex-direction: column;
}

.input-group {
  position: relative;
  margin-bottom: 1.6rem;
}

/* In compact mode the editor stretches to fill the window: the window height
   is the resize target (compact dims), not the editor height setting. */
.input-panel.minimal-panel .input-group {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  margin-bottom: 0;
}

.textarea-wrapper {
  position: relative;
  margin-bottom: 0;
}

.textarea-wrapper.tabs-hidden :deep(.cm-editor) {
  border-radius: 6px;
}

.textarea-wrapper.incoming-wrapper {
  display: flex;
  flex-direction: column;
}

.textarea-wrapper.minimal-wrapper {
  flex: 1 0 140px;
  display: flex;
  flex-direction: column;
  min-height: 140px;
}

.textarea-wrapper.minimal-wrapper :deep(.tts-editor) {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.textarea-wrapper.minimal-wrapper :deep(.cm-editor) {
  flex: 1;
  min-height: 140px;
  box-shadow: none;
}

/* Compact and mono retain actual text focus without a surrounding focus ring. */
.textarea-wrapper.minimal-wrapper :deep(.cm-editor.cm-focused) {
  outline: none;
  border-color: var(--color-border-strong);
  box-shadow: none;
}

.textarea-wrapper.minimal-wrapper :deep(.cm-scroller) {
  min-height: 0 !important;
  height: 100%;
}

/* Minimal + open history: drop the CodeMirror floor to ~80px and give the
   wrapper a floor that accounts for the tab row above the editor, so
   tabs + editor + action bar + history filter fit inside the 300px compact
   viewport. Larger windows grow the wrapper and give the editor more space
   naturally; the history list stays the region that shrinks first and scrolls. */
.input-panel.minimal-panel.history-open .textarea-wrapper.minimal-wrapper {
  flex: 1 0 112px;
  min-height: 112px;
}

.input-panel.minimal-panel.history-open .textarea-wrapper.minimal-wrapper :deep(.cm-editor) {
  min-height: 80px;
}

/* Mono mode: pure clean editor with no headers or footers, filling the entire space */
.input-panel.minimal-panel.mono-panel {
  padding: 0 !important;
  height: 100%;
}

.input-panel.minimal-panel.mono-panel .input-group {
  flex: 1 1 100%;
  height: 100%;
  margin-bottom: 0;
}

.input-panel.minimal-panel.mono-panel .textarea-wrapper.minimal-wrapper {
  flex: 1 1 100%;
  min-height: 0;
  height: 100%;
}

.input-panel.minimal-panel.mono-panel .textarea-wrapper.minimal-wrapper :deep(.cm-editor) {
  min-height: 0 !important;
  height: 100%;
  border: 1px solid rgba(var(--rgb-border), 0.22);
  border-radius: 6px;
}

/* Compact + open history: the editor keeps a floor and the history results
   shrink into the remaining space and scroll instead of squeezing the editor
   flat or pushing it across the action bar / history rows. */
.input-panel.minimal-panel :deep(.phrase-history) {
  flex: 0 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  margin-bottom: 0;
}

.input-panel.minimal-panel :deep(.phrase-panel) {
  flex: 0 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.input-panel.minimal-panel :deep(.filter-row) {
  flex-shrink: 0;
}

.input-panel.minimal-panel :deep(.phrase-list) {
  flex: 1 1 auto;
  min-height: 0;
  max-height: 160px;
  overflow-y: auto;
}

.editor-resize-handle {
  height: 6px;
  cursor: ns-resize;
  background: transparent;
  transition: background 0.15s ease;
  border-radius: 0 0 3px 3px;
  margin-top: -1px;
}

.editor-resize-handle:hover {
  background: var(--color-accent);
  opacity: 0.4;
}

.editor-action-bar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0;
  flex-wrap: nowrap;
  flex-shrink: 0;
}

.editor-action-bar.compact-action-bar {
  padding: 0.25rem 0;
  gap: 0.25rem;
}

.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.3rem 0.75rem;
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  border: 1px solid var(--color-border-strong);
  border-radius: 8px;
  font-size: 0.8rem;
  font-family: var(--font-mono);
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
}

.action-btn:hover:not(:disabled) {
  background: var(--color-accent);
  color: var(--color-text-on-accent, #ffffff);
  border-color: var(--color-accent);
}

.action-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.action-btn.active {
  background: var(--color-accent);
  color: var(--color-text-on-accent, #ffffff);
  border-color: var(--color-accent);
}

/* The history toggle is a state indicator, not an action: no accent fill
   while open (overrides .action-btn.active). */
.action-btn.history-btn.active {
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  border-color: var(--color-border-strong);
}

.ai-btn.loading {
  animation: pulse 1.5s ease-in-out infinite;
}

.speak-btn {
  font-weight: 500;
  flex-shrink: 0;
  background: var(--color-accent);
  color: var(--color-text-on-accent, #ffffff);
  border-color: var(--color-accent);
}

.speak-btn:hover:not(:disabled) {
  filter: brightness(1.1);
}

.speak-btn:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.speak-btn.icon-only {
  padding: 0.3rem 0.55rem;
}

.compact-action-bar .speak-btn {
  margin-right: 16px;
}

.typing-toggle-btn {
  position: relative;
  flex-shrink: 0;
  padding: 0.3rem 0.55rem;
}

/* Neutral hover: the typing toggle is a state indicator, not an action —
   no accent fill on hover (overrides .action-btn:hover). */
.typing-toggle-btn:hover:not(:disabled) {
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  border-color: var(--color-border-strong);
}

.typing-toggle-btn.active {
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  border-color: var(--color-border-strong);
}

.typing-toggle-btn:not(.active) {
  opacity: 0.45;
}

.typing-toggle-btn:not(.active)::after {
  content: '';
  position: absolute;
  left: 50%;
  top: 50%;
  width: 70%;
  height: 1.5px;
  background: currentColor;
  border-radius: 1px;
  transform: translate(-50%, -50%) rotate(-45deg);
}

.action-bar-spacer {
  flex: 1;
  min-width: 0;
}

/* Ambient indicator of the quick-editor mode (collapse / return focus);
   hidden entirely when the mode is disabled. Not a control. */
.quick-mode-indicator {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--color-text-secondary);
  opacity: 0.8;
  margin-right: 0.35rem;
}

@media (max-width: 960px) {
  .input-panel {
    padding-bottom: 1.5rem;
  }
}

.ai-editor-hint {
  margin-top: 0.5rem;
  font-size: var(--ui-text-size-hint);
  color: var(--color-accent);
  opacity: 0.8;
  text-align: center;
  font-weight: var(--ui-text-weight-hint);
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.6; }
}
</style>
