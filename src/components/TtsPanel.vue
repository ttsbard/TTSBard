<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, inject } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { createAsyncCleanupScope } from '../utils/asyncCleanup';
import { useTtsSettings, useAppSettings } from '../composables/useAppSettings';
import type {
  ElevenLabsGenerationSettingsInput,
  ElevenLabsModel,
  ElevenLabsVoice,
  FishAudioConnectionSettingsInput,
  TtsProviderType,
  TtsProviderInfoDto,
  VoiceModel,
} from '../types/settings';
import { debugLog, debugError } from '../utils/debug';
import { t } from '../i18n';
import { LocalizedError, presentCommandError } from '../ipc/commandError';
import { useErrorHandler } from '../composables/useErrorHandler';
import { Ellipsis } from 'lucide-vue-next';
import { isPiperPhonemizationError } from './ttsProviderSelection';
import { TELEGRAM_AUTH_KEY, type UseTelegramAuthReturn } from '../composables/useTelegramAuth';
import TelegramAuthModal from './TelegramAuthModal.vue';
import StatusMessage from './shared/StatusMessage.vue';
import TtsSileroCard from './tts/TtsSileroCard.vue';
import TtsLocalCard from './tts/TtsLocalCard.vue';
import TtsPocketCard from './tts/TtsPocketCard.vue';
import TtsPiperCard from './tts/TtsPiperCard.vue';
import TtsOpenAICard from './tts/TtsOpenAICard.vue';
import TtsFishAudioCard from './tts/TtsFishAudioCard.vue';
import TtsElevenLabsCard from './tts/TtsElevenLabsCard.vue';
import {
  BUILTIN_PROVIDER_ID_BY_TYPE,
  PIPER_VISIBILITY_ID,
  deriveLegacyVisibleIds,
  forceActiveVisible,
  isPersistedVisibility,
  normalizeVisibleIds,
  normalizeVisibilityId,
  toggleVisibleId,
} from './ttsProviderVisibility';
import {
  selectBuiltinTtsProvider,
  selectConcreteTtsProvider,
} from './ttsProviderSelection';

interface TtsProviderState {
  type: TtsProviderType;
  configured: boolean;
  expanded: boolean;
}

// State
const { showError: showGlobalError } = useErrorHandler();
const activeProvider = ref<TtsProviderType | null>(null);
const activeProviderId = ref<string | null>(null);

const providers = ref<Record<TtsProviderType, TtsProviderState>>({
  openai: { type: 'openai', configured: false, expanded: false },
  silero: { type: 'silero', configured: false, expanded: false },
  local: { type: 'local', configured: false, expanded: false },
  fish: { type: 'fish', configured: false, expanded: false },
  elevenlabs: { type: 'elevenlabs', configured: false, expanded: false },
});

// Get settings from composable
const ttsSettings = useTtsSettings();
const { settings: appSettings, reload: reloadSettings } = useAppSettings();

const speakerDevice = computed(() => appSettings.value?.audio.speaker_device ?? null);
const speakerVolume = computed(() => appSettings.value?.audio.speaker_volume ?? 100);

// OpenAI settings
const openaiApiKey = ref('');
const openaiVoice = ref('alloy');
const openaiVoices = ['alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'];
const openaiUseProxy = ref(false);

// local TTS settings
const localTtsUrl = ref('http://127.0.0.1:8124');

// Fish Audio settings
const fishAudioApiKey = ref('');
const fishAudioReferenceId = ref('');
const fishAudioVoices = ref<VoiceModel[]>([]);
const fishAudioFormat = ref('mp3');
const fishAudioTemperature = ref(0.7);
const fishAudioSampleRate = ref(44100);
const fishAudioUseProxy = ref(false);

// ElevenLabs settings
const elevenLabsApiKey = ref('');
const elevenLabsVoiceId = ref('');
const elevenLabsVoices = ref<ElevenLabsVoice[]>([]);
const elevenLabsModels = ref<ElevenLabsModel[]>([]);
const elevenLabsModelId = ref('');
const elevenLabsOutputFormat = ref('mp3_44100_128');
const elevenLabsStability = ref(0.5);
const elevenLabsSimilarityBoost = ref(0.75);
const elevenLabsStyle = ref(0);
const elevenLabsUseSpeakerBoost = ref(true);
const elevenLabsUseProxy = ref(false);
const elevenLabsFirstLoadLoading = ref(false);
const elevenLabsModelsLoading = ref(false);
const elevenLabsModelsError = ref<string | null>(null);
const elevenLabsVoicesLoading = ref(false);
const elevenLabsVoicesError = ref<string | null>(null);
let elevenLabsModelsGeneration = 0;
let elevenLabsVoicesGeneration = 0;

// Piper runtime providers
const piperProviders = ref<TtsProviderInfoDto[]>([]);
const piperLoading = ref<Record<string, boolean>>({});
const piperError = ref<Record<string, string | null>>({});
const activePiperId = ref<string | null>(null);

// Pocket TTS active concrete provider. The card discovers its own model list;
// only the persisted active ID is owned by this panel.
const activePocketId = ref<string | null>(null);

// Provider visibility
const visibilityOpen = ref(false);
const visibilityButtonRef = ref<HTMLButtonElement | null>(null);
const visibilityPopoverRef = ref<HTMLElement | null>(null);
const visibilityOverride = ref<string[] | null>(null);
// One visibility entry controls the card regardless of the selected Pocket model.
const POCKET_VISIBILITY_ID = 'pocket';

const cloudVisibilityEntries = computed(() => [
  { id: BUILTIN_PROVIDER_ID_BY_TYPE.silero, label: t('tts.providers.silero') },
  { id: BUILTIN_PROVIDER_ID_BY_TYPE.openai, label: t('tts.providers.openai') },
  { id: BUILTIN_PROVIDER_ID_BY_TYPE.fish, label: t('tts.providers.fish') },
  { id: BUILTIN_PROVIDER_ID_BY_TYPE.elevenlabs, label: t('tts.providers.elevenlabs') },
]);

const localVisibilityEntries = computed(() => [
  { id: BUILTIN_PROVIDER_ID_BY_TYPE.local, label: t('tts.local.title') },
  { id: POCKET_VISIBILITY_ID, label: t('tts.pocket.title') },
  { id: PIPER_VISIBILITY_ID, label: t('tts.piper.title') },
]);

const configuredProviderIds = computed<string[]>(() => {
  const settings = ttsSettings.value;
  if (!settings) return [];
  const ids: string[] = [];
  if (settings.openai?.api_key) ids.push(BUILTIN_PROVIDER_ID_BY_TYPE.openai);
  if (settings.local?.url) ids.push(BUILTIN_PROVIDER_ID_BY_TYPE.local);
  if (settings.fish?.api_key) ids.push(BUILTIN_PROVIDER_ID_BY_TYPE.fish);
  if (settings.elevenlabs?.api_key) ids.push(BUILTIN_PROVIDER_ID_BY_TYPE.elevenlabs);
  return ids;
});

const piperIds = computed<string[]>(() => piperProviders.value.map(p => p.id));

const baseVisibleIds = computed<string[]>(() => {
  const persisted = ttsSettings.value?.visible_provider_ids;
  if (isPersistedVisibility(persisted)) {
    return normalizeVisibleIds(persisted as string[]);
  }
  return deriveLegacyVisibleIds({
    activeProviderId: activeProviderId.value,
    configuredIds: configuredProviderIds.value,
    piperIds: piperIds.value,
  });
});

const visibilityActiveId = computed(() => {
  if (activePocketId.value) return POCKET_VISIBILITY_ID;
  if (
    activePiperId.value
    || (activeProviderId.value && normalizeVisibilityId(activeProviderId.value) === PIPER_VISIBILITY_ID)
  ) {
    return PIPER_VISIBILITY_ID;
  }
  return activeProviderId.value;
});
const visibleIds = computed<string[]>(() =>
  forceActiveVisible(visibilityOverride.value ?? baseVisibleIds.value, visibilityActiveId.value),
);

const sileroVisible = computed(() => visibleIds.value.includes(BUILTIN_PROVIDER_ID_BY_TYPE.silero));
const openaiVisible = computed(() => visibleIds.value.includes(BUILTIN_PROVIDER_ID_BY_TYPE.openai));
const fishVisible = computed(() => visibleIds.value.includes(BUILTIN_PROVIDER_ID_BY_TYPE.fish));
const elevenLabsVisible = computed(() => visibleIds.value.includes(BUILTIN_PROVIDER_ID_BY_TYPE.elevenlabs));
const localVisible = computed(() => visibleIds.value.includes(BUILTIN_PROVIDER_ID_BY_TYPE.local));

const piperVisible = computed(() => visibleIds.value.includes(PIPER_VISIBILITY_ID));

// Telegram auth
const showTelegramModal = ref(false);
const telegramAuth = inject<UseTelegramAuthReturn>(TELEGRAM_AUTH_KEY)!;
const {
  status: telegramStatus,
  isConnected: telegramConnected,
  errorMessage: telegramErrorMessage,
  hasError: telegramHasError,
  signOut: signOutTelegram,
  currentVoice: telegramCurrentVoice,
  savedVoices: telegramSavedVoices,
  voiceLoading: telegramVoiceLoading,
  voiceError: telegramVoiceError,
  addVoiceCode: addTelegramVoiceCode,
  removeVoiceCode: removeTelegramVoiceCode,
  selectVoice: selectTelegramVoice,
  loadSavedVoices: loadTelegramSavedVoices,
  autoRefreshVoice: autoRefreshTelegramVoice,
  limits: telegramLimits,
  limitsLoading: telegramLimitsLoading,
  limitsError: telegramLimitsError,
  refreshLimits: refreshTelegramLimits,
} = telegramAuth;

// silero error state
const sileroError = ref<string | null>(null);

// Telegram proxy state
const telegramProxyMode = ref<string>('none');
const telegramProxyModes = [
  { value: 'none', label: t('tts.proxy.none') },
  { value: 'socks5', label: 'SOCKS5' },
  { value: 'mtproxy', label: 'MTProxy' }
];

// Telegram reconnection state
const reconnectingTelegram = ref(false);

// Current Telegram proxy status (from backend)
const currentTelegramProxyStatus = ref<{
  mode: string
  proxy_url: string | null
} | null>(null);

// Error state
const statusMessage = ref('');
const statusType = ref<'success' | 'error'>('error');
let statusTimeout: ReturnType<typeof setTimeout> | null = null;
let errorTimeout: ReturnType<typeof setTimeout> | null = null;
const listenerScope = createAsyncCleanupScope();

// Methods
function showStatus(message: string, type: 'success' | 'error' = 'error') {
  statusMessage.value = message;
  statusType.value = type;
  if (statusTimeout) clearTimeout(statusTimeout);
  statusTimeout = setTimeout(() => {
    statusMessage.value = '';
  }, 3000);
}

function showError(message: string) {
  showStatus(message, 'error');
}

function showSuccess(message: string) {
  showStatus(message, 'success');
}

function toggleVisibilityPopover() {
  visibilityOpen.value = !visibilityOpen.value;
}

function closeVisibilityPopover() {
  visibilityOpen.value = false;
}

function handleVisibilityDocumentClick(event: MouseEvent) {
  if (!visibilityOpen.value) return;
  const target = event.target as Node;
  if (visibilityPopoverRef.value?.contains(target)) return;
  if (visibilityButtonRef.value?.contains(target)) return;
  closeVisibilityPopover();
}

function handleVisibilityDocumentKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !visibilityOpen.value) return;
  closeVisibilityPopover();
  visibilityButtonRef.value?.focus();
}

async function onVisibilityToggle(id: string) {
  if (id === visibilityActiveId.value) return;
  const previous = visibilityOverride.value;
  const next = toggleVisibleId(visibleIds.value, id, visibilityActiveId.value);
  visibilityOverride.value = next;
  try {
    await invoke('set_visible_tts_provider_ids', { providerIds: next });
  } catch (error) {
    visibilityOverride.value = previous;
    showError(presentCommandError(error, t('tts.error.set_visibility')));
  }
}

function toggleProvider(provider: TtsProviderType) {
  providers.value[provider].expanded = !providers.value[provider].expanded;
}

async function saveOpenAiApiKey(key: string) {
  debugLog('[TTS] Saving OpenAI API key...');

  if (!key.trim()) {
    showError(t('tts.error.api_key_required'));
    return;
  }

  try {
    await invoke('set_openai_api_key', { key });
    providers.value.openai.configured = true;
    debugLog('[TTS] OpenAI API key saved successfully');
    showSuccess(t('tts.api_key.saved'));
  } catch (error) {
    debugError('[TTS] Failed to save OpenAI API key:', error);
    showError(presentCommandError(error, t('tts.error.save_api_key')));
  }
}

async function saveOpenAiVoice(voice: string) {
  debugLog('[TTS] Saving OpenAI voice:', voice);
  try {
    await invoke('set_openai_voice', { voice });
    debugLog('[TTS] OpenAI voice saved successfully:', voice);
    showSuccess(t('tts.voice.saved', { voice }));
  } catch (error) {
    debugError('[TTS] Failed to save OpenAI voice:', error);
    showError(presentCommandError(error, t('tts.error.save_voice')));
  }
}

async function toggleOpenAiUseProxy(enabled: boolean) {
  try {
    await invoke('set_openai_use_proxy', { enabled });
    debugLog('[TTS] OpenAI use proxy toggled:', enabled);

    if (activeProvider.value === 'openai') {
      await invoke('apply_openai_proxy_settings');
      debugLog('[TTS] Applied proxy settings to OpenAI provider');
    }

    showSuccess(enabled ? t('tts.proxy.enabled') : t('tts.proxy.disabled'));
  } catch (error) {
    debugError('[TTS] Failed to toggle OpenAI proxy:', error);
    showError(presentCommandError(error, t('tts.error.toggle_proxy')));
    // Revert on error - the parent will handle this
    throw error;
  }
}

async function saveLocalTtsUrl(url: string) {
  try {
    await invoke('set_local_tts_url', { url });
    localTtsUrl.value = url;
    providers.value.local.configured = true;
    showSuccess(t('tts.url.saved'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.save_url')));
  }
}

async function saveFishAudioSettings(data: FishAudioConnectionSettingsInput): Promise<void> {
  debugLog('[TTS] Saving Fish Audio settings...');

  if (!data.apiKey.trim()) {
    showError(t('tts.error.api_key_required'));
    throw new LocalizedError(t('tts.error.api_key_required'));
  }

  try {
    await invoke('save_fish_audio_connection_settings', {
      settings: {
        apiKey: data.apiKey,
        format: data.format,
        temperature: data.temperature,
        sampleRate: data.sampleRate,
      },
    });

    providers.value.fish.configured = true;
    await reloadSettings();
    debugLog('[TTS] Fish Audio settings saved successfully');
    showSuccess(t('tts.settings.saved'));
  } catch (error) {
    debugError('[TTS] Failed to save Fish Audio settings:', error);
    showError(presentCommandError(error, t('tts.error.save_fish_settings')));
    throw error;
  }
}

async function saveFishAudioReferenceId(referenceId: string) {
  try {
    await invoke('set_fish_audio_reference_id', { referenceId });
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.save_fish_reference_id')));
  }
}

async function addFishAudioVoice(model: VoiceModel) {
  try {
    await invoke('add_fish_audio_voice', { voice: model });
    await reloadSettings();
    showSuccess(t('tts.voice_model.added'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.add_fish_voice')));
  }
}

async function removeFishAudioVoice(voiceId: string) {
  try {
    await invoke('remove_fish_audio_voice', { voiceId });
    await reloadSettings();
    showSuccess(t('tts.voice_model.removed'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.remove_fish_voice')));
  }
}

async function selectFishAudioVoice(voiceId: string) {
  await saveFishAudioReferenceId(voiceId);
  await reloadSettings();
}

async function toggleFishAudioUseProxy(enabled: boolean) {
  try {
    await invoke('set_fish_audio_use_proxy', { enabled });

    if (activeProvider.value === 'fish') {
      await invoke('apply_fish_audio_proxy_settings');
    }

    showSuccess(enabled ? t('tts.proxy.enabled') : t('tts.proxy.disabled'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.toggle_proxy')));
    throw error;
  }
}

async function saveElevenLabsApiKey(key: string): Promise<void> {
  debugLog('[TTS] Saving ElevenLabs API key...');

  if (!key.trim()) {
    showError(t('tts.error.api_key_required'));
    throw new LocalizedError(t('tts.error.api_key_required'));
  }

  try {
    const changed = await invoke<boolean>('save_elevenlabs_api_key', { key });
    providers.value.elevenlabs.configured = true;
    await reloadSettings();
    debugLog('[TTS] ElevenLabs API key saved successfully');
    showSuccess(t('tts.api_key.saved'));

    // Auto-load account catalogs after a key save. The key-change result and the
    // freshly reloaded catalogs decide which requests are actually needed.
    await firstLoadElevenLabsCatalogs(changed);
  } catch (error) {
    debugError('[TTS] Failed to save ElevenLabs API key:', error);
    showError(presentCommandError(error, t('tts.error.save_api_key')));
    throw error;
  }
}

async function saveElevenLabsGenerationSettings(data: ElevenLabsGenerationSettingsInput): Promise<void> {
  debugLog('[TTS] Saving ElevenLabs generation settings...');

  try {
    await invoke('save_elevenlabs_generation_settings', {
      settings: {
        modelId: data.modelId,
        outputFormat: data.outputFormat,
        stability: data.stability,
        similarityBoost: data.similarityBoost,
        style: data.style,
        useSpeakerBoost: data.useSpeakerBoost,
      },
    });

    // Reload so the normalized backend values become the new baseline.
    await reloadSettings();
    debugLog('[TTS] ElevenLabs generation settings saved successfully');
    showSuccess(t('tts.settings.saved'));
  } catch (error) {
    debugError('[TTS] Failed to save ElevenLabs generation settings:', error);
    showError(presentCommandError(error, t('tts.error.save_elevenlabs_settings')));
    throw error;
  }
}

async function firstLoadElevenLabsCatalogs(keyChanged: boolean): Promise<void> {
  const modelsEmpty = elevenLabsModels.value.length === 0;
  const voicesEmpty = elevenLabsVoices.value.length === 0;
  const loadModels = keyChanged || modelsEmpty;
  const loadVoices = keyChanged || voicesEmpty;

  const tasks: Promise<void>[] = [];
  if (loadModels) tasks.push(refreshElevenLabsModels());
  if (loadVoices) tasks.push(refreshElevenLabsVoices());
  if (tasks.length === 0) return;

  elevenLabsFirstLoadLoading.value = true;
  try {
    await Promise.allSettled(tasks);
  } finally {
    elevenLabsFirstLoadLoading.value = false;
  }
}

async function refreshElevenLabsModels(): Promise<void> {
  if (elevenLabsModelsLoading.value) return;

  elevenLabsModelsLoading.value = true;
  elevenLabsModelsError.value = null;
  const generation = ++elevenLabsModelsGeneration;

  try {
    await invoke<ElevenLabsModel[]>('refresh_elevenlabs_models');

    // Ignore a stale completion after the panel state has been replaced.
    if (generation !== elevenLabsModelsGeneration) return;

    // The refresh command returns the catalog only; reload settings to obtain
    // the backend-selected model_id and keep the catalog UI consistent.
    await reloadSettings();
  } catch (error) {
    if (generation !== elevenLabsModelsGeneration) return;
    elevenLabsModelsError.value = presentCommandError(error, t('tts.error.refresh_elevenlabs_models'));
  } finally {
    if (generation === elevenLabsModelsGeneration) {
      elevenLabsModelsLoading.value = false;
    }
  }
}

async function refreshElevenLabsVoices(): Promise<void> {
  if (elevenLabsVoicesLoading.value) return;

  elevenLabsVoicesLoading.value = true;
  elevenLabsVoicesError.value = null;
  const generation = ++elevenLabsVoicesGeneration;

  try {
    await invoke<ElevenLabsVoice[]>('refresh_elevenlabs_voices');

    // Ignore a stale completion after the panel state has been replaced.
    if (generation !== elevenLabsVoicesGeneration) return;

    // The refresh command returns the catalog only; reload settings to obtain
    // the backend-selected voice_id and keep the catalog UI consistent.
    await reloadSettings();
  } catch (error) {
    if (generation !== elevenLabsVoicesGeneration) return;
    elevenLabsVoicesError.value = presentCommandError(error, t('tts.error.refresh_elevenlabs_voices'));
  } finally {
    if (generation === elevenLabsVoicesGeneration) {
      elevenLabsVoicesLoading.value = false;
    }
  }
}

async function selectElevenLabsVoice(voiceId: string) {
  if (!voiceId) return;

  try {
    await invoke('set_elevenlabs_voice_id', { voiceId });
    await reloadSettings();
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.select_voice')));
  }
}

async function toggleElevenLabsUseProxy(enabled: boolean) {
  try {
    await invoke('set_elevenlabs_use_proxy', { enabled });

    if (activeProvider.value === 'elevenlabs') {
      await invoke('apply_elevenlabs_proxy_settings');
    }

    showSuccess(enabled ? t('tts.proxy.enabled') : t('tts.proxy.disabled'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.toggle_proxy')));
    throw error;
  }
}

async function setActiveProvider(provider: TtsProviderType) {
  try {
    await selectBuiltinTtsProvider(provider);
    activeProvider.value = provider;
    activePiperId.value = null;
    activePocketId.value = null;
    activeProviderId.value = BUILTIN_PROVIDER_ID_BY_TYPE[provider];
    await reloadSettings();
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.select_provider')));
  }
}

async function selectPiperProvider(id: string) {
  if (piperLoading.value[id]) return;

  piperLoading.value[id] = true;
  piperError.value[id] = null;

  try {
    await selectConcreteTtsProvider(id);
    activeProvider.value = null;
    activePiperId.value = id;
    activePocketId.value = null;
    activeProviderId.value = id;
    showSuccess(t('tts.model.loaded'));
  } catch (error) {
    piperError.value[id] = presentCommandError(error, t('tts.model.load_error'));
    const message = piperError.value[id];
    if (isPiperPhonemizationError(error)) showGlobalError(message);
    else showError(message);
  } finally {
    piperLoading.value[id] = false;
    await reloadSettings();
  }
}

async function handlePocketActivated(id: string) {
  activeProvider.value = null;
  activePiperId.value = null;
  activePocketId.value = id;
  activeProviderId.value = id;
  await reloadSettings();
}

// The card owns the refresh request and hands back the backend snapshot, so the
// list (and its empty state) updates as soon as the rescan completes.
function applyPiperSnapshot(providers: TtsProviderInfoDto[]) {
  piperProviders.value = Array.isArray(providers) ? providers : [];
}

function openTelegramModal() {
  showTelegramModal.value = true;
  sileroError.value = null;
}

async function handleSileroError() {
  if (telegramHasError.value && telegramErrorMessage.value) {
    sileroError.value = telegramErrorMessage.value;
  }
}

async function handleSignOut() {
  await signOutTelegram();
  sileroError.value = null;
}

async function loadTelegramProxyStatus() {
  if (!telegramConnected.value) {
    currentTelegramProxyStatus.value = null;
    return;
  }

  try {
    const status = await invoke<{
      mode: string
      proxy_url: string | null
    }>('get_telegram_proxy_status');
    currentTelegramProxyStatus.value = status;
    debugLog('[TTS] Telegram proxy status loaded:', status);
  } catch (error) {
    debugError('[TTS] Failed to load Telegram proxy status:', error);
    currentTelegramProxyStatus.value = null;
  }
}

async function setSileroTimeout(timeoutMs: number) {
  try {
    await invoke('set_silero_synthesis_timeout', { timeoutMs });
    if (ttsSettings.value?.telegram) {
      ttsSettings.value.telegram.synthesis_response_timeout_ms = timeoutMs;
    }
    showSuccess(t('tts.silero.timeout.saved'));
  } catch (error) {
    debugError('[TTS] Failed to save Silero timeout:', error);
    showError(presentCommandError(error, t('tts.silero.timeout.error')));
  }
}

async function setTelegramProxyMode(mode: string) {
  telegramProxyMode.value = mode;
  try {
    await invoke('set_telegram_proxy_mode', { mode });
    debugLog('[TTS] Telegram proxy mode saved:', mode);
  } catch (error) {
    debugError('[TTS] Failed to save proxy mode:', error);
  }
}

async function reconnectTelegram() {
  reconnectingTelegram.value = true;

  try {
    await invoke('set_telegram_proxy_mode', { mode: telegramProxyMode.value });
    debugLog('[TTS] Telegram proxy mode saved before reconnect:', telegramProxyMode.value);

    const result = await invoke<string>('reconnect_telegram');
    debugLog('[TTS] Telegram reconnected:', result);

    await loadTelegramProxyStatus();
    void refreshTelegramLimits();

    showSuccess(t('tts.telegram.reconnected'));
  } catch (error) {
    debugError('[TTS] Failed to reconnect Telegram:', error);
    showError(presentCommandError(error, t('tts.error.reconnect_telegram')));
  } finally {
    reconnectingTelegram.value = false;
  }
}

// Voice management handlers
async function handleRefreshVoice() {
  try {
    await autoRefreshTelegramVoice();
    showSuccess(t('tts.telegram.voice_updated'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.refresh_voice')));
  }
}

async function handleAddVoice(data: { code: string; description?: string }, callback: (success: boolean, error?: string) => void) {
  try {
    await addTelegramVoiceCode(data);
    showSuccess(t('tts.telegram.voice_added'));
    callback(true);
  } catch (error) {
    const errorMsg = presentCommandError(error, t('tts.silero.add_voice.error'));
    // Ошибка уже покажется в диалоге, не дублируем
    callback(false, errorMsg);
  }
}

async function handleRemoveVoice(id: string) {
  try {
    await removeTelegramVoiceCode(id);
    showSuccess(t('tts.telegram.voice_removed'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.remove_voice')));
  }
}

async function handleSelectVoice(id: string) {
  try {
    await selectTelegramVoice(id);
    showSuccess(t('tts.telegram.voice_selected'));
  } catch (error) {
    showError(presentCommandError(error, t('tts.error.select_voice')));
  }
}

// Watch for Telegram errors
watch([telegramErrorMessage, telegramHasError], () => {
  handleSileroError();
});

// Clear silero error when successfully connected
watch(telegramConnected, async (newValue) => {
  if (newValue) {
    sileroError.value = null;
    void refreshTelegramLimits();
    void loadTelegramProxyStatus();
    await loadTelegramSavedVoices();
    await autoRefreshTelegramVoice();
  } else {
    currentTelegramProxyStatus.value = null;
  }
}, { immediate: true });

// Save proxy mode when user opens Telegram connection modal
watch(showTelegramModal, async (isOpen) => {
  if (isOpen) {
    try {
      await invoke('set_telegram_proxy_mode', { mode: telegramProxyMode.value });
      debugLog('[TTS] Telegram proxy mode saved before connection:', telegramProxyMode.value);
    } catch (error) {
      debugError('[TTS] Failed to save proxy mode:', error);
    }
  }
});

// Watch for settings changes from composable
watch(ttsSettings, (newSettings) => {
  if (!newSettings) return;

  if (isPersistedVisibility(newSettings.visible_provider_ids)) {
    visibilityOverride.value = null;
  }

  debugLog('[TTS] Settings updated from composable:', {
    provider: newSettings.provider,
    has_openai: !!newSettings.openai,
    has_local: !!newSettings.local,
    has_fish: !!newSettings.fish,
    has_telegram: !!newSettings.telegram,
  });

  if (newSettings.providers) {
    piperProviders.value = newSettings.providers.filter(p => p.kind === 'piper');
    const activePiper = newSettings.providers.find(p => p.kind === 'piper' && p.active);
    const activePocket = newSettings.providers.find(p => p.kind === 'pocket' && p.active);
    activePiperId.value = activePiper?.id ?? null;
    activePocketId.value = activePocket?.id ?? null;
  }

  if (activePocketId.value) {
    activeProvider.value = null;
    activePiperId.value = null;
    activeProviderId.value = activePocketId.value;
  } else if (activePiperId.value) {
    activeProvider.value = null;
    activeProviderId.value = activePiperId.value;
  } else if (newSettings.provider) {
    debugLog('[TTS] Setting activeProvider to:', newSettings.provider);
    activeProvider.value = newSettings.provider;
    activeProviderId.value = newSettings.provider_id ?? BUILTIN_PROVIDER_ID_BY_TYPE[newSettings.provider];
  }

  if (newSettings.openai) {
    if (newSettings.openai.api_key) {
      openaiApiKey.value = newSettings.openai.api_key;
      providers.value.openai.configured = true;
    }
    if (newSettings.openai.voice) {
      openaiVoice.value = newSettings.openai.voice;
    }
    if (newSettings.openai.use_proxy !== undefined) {
      openaiUseProxy.value = newSettings.openai.use_proxy;
    }
  }

  if (newSettings.telegram?.proxy_mode) {
    telegramProxyMode.value = newSettings.telegram.proxy_mode;
  }

  if (newSettings.local && newSettings.local.url) {
    localTtsUrl.value = newSettings.local.url;
    providers.value.local.configured = newSettings.local.url.length > 0;
  }

  if (newSettings.fish) {
    if (newSettings.fish.api_key) {
      fishAudioApiKey.value = newSettings.fish.api_key;
      providers.value.fish.configured = true;
    }
    if (newSettings.fish.reference_id) {
      fishAudioReferenceId.value = newSettings.fish.reference_id;
    }
    if (newSettings.fish.voices) {
      fishAudioVoices.value = newSettings.fish.voices;
    }
    if (newSettings.fish.format) {
      fishAudioFormat.value = newSettings.fish.format;
    }
    if (newSettings.fish.temperature !== undefined) {
      fishAudioTemperature.value = newSettings.fish.temperature;
    }
    if (newSettings.fish.sample_rate) {
      fishAudioSampleRate.value = newSettings.fish.sample_rate;
    }
    if (newSettings.fish.use_proxy !== undefined) {
      fishAudioUseProxy.value = newSettings.fish.use_proxy;
    }
  }

  if (newSettings.elevenlabs) {
    const el = newSettings.elevenlabs;
    // Do not erase a locally entered key merely because a transient DTO value
    // is null; only overwrite when the backend reports a real key.
    if (el.api_key) {
      elevenLabsApiKey.value = el.api_key;
      providers.value.elevenlabs.configured = true;
    }
    if (el.voice_id) {
      elevenLabsVoiceId.value = el.voice_id;
    }
    if (el.voices) {
      elevenLabsVoices.value = el.voices;
    }
    if (el.models) {
      elevenLabsModels.value = el.models;
    }
    if (el.model_id !== undefined) {
      elevenLabsModelId.value = el.model_id;
    }
    if (el.output_format) {
      elevenLabsOutputFormat.value = el.output_format;
    }
    if (el.stability !== undefined) {
      elevenLabsStability.value = el.stability;
    }
    if (el.similarity_boost !== undefined) {
      elevenLabsSimilarityBoost.value = el.similarity_boost;
    }
    if (el.style !== undefined) {
      elevenLabsStyle.value = el.style;
    }
    if (el.use_speaker_boost !== undefined) {
      elevenLabsUseSpeakerBoost.value = el.use_speaker_boost;
    }
    if (el.use_proxy !== undefined) {
      elevenLabsUseProxy.value = el.use_proxy;
    }
  }
}, { immediate: true, deep: true });

// Load on mount
onMounted(async () => {
  document.addEventListener('click', handleVisibilityDocumentClick);
  document.addEventListener('keydown', handleVisibilityDocumentKeydown);
  await listenerScope.track(listen('tts-error', (event) => {
    showError(event.payload as string);
  }));
});

onUnmounted(() => {
  // Invalidate in-flight ElevenLabs catalog responses before teardown.
  elevenLabsModelsGeneration += 1;
  elevenLabsVoicesGeneration += 1;
  if (errorTimeout) clearTimeout(errorTimeout);
  document.removeEventListener('click', handleVisibilityDocumentClick);
  document.removeEventListener('keydown', handleVisibilityDocumentKeydown);
  listenerScope.dispose();
});

function dismissStatus() {
  statusMessage.value = '';
}
</script>

<template>
  <div class="tts-panel">
    <!-- Status Message -->
    <StatusMessage
      :message="statusMessage"
      :type="statusType"
      @dismiss="dismissStatus"
    />

    <!-- Provider visibility control -->
    <div class="visibility-control">
      <button
        ref="visibilityButtonRef"
        type="button"
        class="ui-icon-button ui-icon-button--round visibility-button"
        :title="t('tts.visibility.configure')"
        :aria-label="t('tts.visibility.configure')"
        @click="toggleVisibilityPopover"
      >
        <Ellipsis :size="18" />
      </button>

      <div
        v-if="visibilityOpen"
        ref="visibilityPopoverRef"
        class="visibility-popover"
      >
        <div class="visibility-group">
          <div class="visibility-group-label">{{ t('tts.visibility.cloud') }}</div>
          <label
            v-for="entry in cloudVisibilityEntries"
            :key="entry.id"
            class="visibility-entry"
          >
            <input
              type="checkbox"
              :checked="visibleIds.includes(entry.id)"
              :disabled="entry.id === activeProviderId"
              @change="onVisibilityToggle(entry.id)"
            />
            <span class="visibility-entry-label ui-choice-label">{{ entry.label }}</span>
            <span v-if="entry.id === activeProviderId" class="visibility-active">{{ t('tts.visibility.active') }}</span>
          </label>
        </div>

        <div class="visibility-group">
          <div class="visibility-group-label">{{ t('tts.visibility.local') }}</div>
          <label
            v-for="entry in localVisibilityEntries"
            :key="entry.id"
            class="visibility-entry"
          >
            <input
              type="checkbox"
              :checked="visibleIds.includes(entry.id)"
              :disabled="entry.id === visibilityActiveId"
              @change="onVisibilityToggle(entry.id)"
            />
            <span class="visibility-entry-label ui-choice-label">{{ entry.label }}</span>
            <span v-if="entry.id === visibilityActiveId" class="visibility-active">{{ t('tts.visibility.active') }}</span>
          </label>
        </div>
      </div>
    </div>

    <!-- Provider Cards -->
    <div class="provider-cards">
      <!-- Silero Provider -->
      <TtsSileroCard
        v-if="sileroVisible || activeProvider === 'silero'"
        :active="activeProvider === 'silero'"
        :expanded="providers.silero.expanded"
        :connected="telegramConnected"
        :telegram-status="telegramStatus"
        :current-proxy-status="currentTelegramProxyStatus"
        :error-message="sileroError"
        :reconnecting="reconnectingTelegram"
        :proxy-mode="telegramProxyMode"
        :proxy-modes="telegramProxyModes"
        :current-voice="telegramCurrentVoice"
        :saved-voices="telegramSavedVoices"
        :voice-loading="telegramVoiceLoading"
        :voice-error="telegramVoiceError"
        :limits="telegramLimits"
        :limits-loading="telegramLimitsLoading"
        :limits-error="telegramLimitsError"
        :synthesis-response-timeout-ms="ttsSettings?.telegram.synthesis_response_timeout_ms"
        @select="setActiveProvider('silero')"
        @toggle="toggleProvider('silero')"
        @connect="openTelegramModal"
        @disconnect="handleSignOut"
        @reconnect="reconnectTelegram"
        @proxy-mode-change="setTelegramProxyMode"
        @refresh-voice="handleRefreshVoice"
        @add-voice="handleAddVoice"
        @remove-voice="handleRemoveVoice"
        @select-voice="handleSelectVoice"
        @refresh-limits="refreshTelegramLimits"
        @timeout-change="setSileroTimeout"
      />

      <!-- OpenAI Provider -->
      <TtsOpenAICard
        v-if="openaiVisible || activeProvider === 'openai'"
        :active="activeProvider === 'openai'"
        :expanded="providers.openai.expanded"
        :api-key="openaiApiKey"
        :voice="openaiVoice"
        :voices="openaiVoices"
        :use-proxy="openaiUseProxy"
        @select="setActiveProvider('openai')"
        @toggle="toggleProvider('openai')"
        @save-api-key="saveOpenAiApiKey"
        @voice-change="saveOpenAiVoice"
        @toggle-proxy="toggleOpenAiUseProxy"
      />

      <!-- Fish Audio Provider -->
      <TtsFishAudioCard
        v-if="fishVisible || activeProvider === 'fish'"
        :active="activeProvider === 'fish'"
        :expanded="providers.fish.expanded"
        :api-key="fishAudioApiKey"
        :reference-id="fishAudioReferenceId"
        :voices="fishAudioVoices"
        :format="fishAudioFormat"
        :temperature="fishAudioTemperature"
        :sample-rate="fishAudioSampleRate"
        :use-proxy="fishAudioUseProxy"
        @select="setActiveProvider('fish')"
        @toggle="toggleProvider('fish')"
        :on-save-all="saveFishAudioSettings"
        @select-voice="selectFishAudioVoice"
        @add-voice="addFishAudioVoice"
        @remove-voice="removeFishAudioVoice"
        @toggle-proxy="toggleFishAudioUseProxy"
      />

      <!-- ElevenLabs Provider -->
      <TtsElevenLabsCard
        v-if="elevenLabsVisible || activeProvider === 'elevenlabs'"
        :active="activeProvider === 'elevenlabs'"
        :expanded="providers.elevenlabs.expanded"
        :api-key="elevenLabsApiKey"
        :voice-id="elevenLabsVoiceId"
        :voices="elevenLabsVoices"
        :models="elevenLabsModels"
        :model-id="elevenLabsModelId"
        :output-format="elevenLabsOutputFormat"
        :stability="elevenLabsStability"
        :similarity-boost="elevenLabsSimilarityBoost"
        :style="elevenLabsStyle"
        :use-speaker-boost="elevenLabsUseSpeakerBoost"
        :use-proxy="elevenLabsUseProxy"
        :models-loading="elevenLabsModelsLoading"
        :models-error="elevenLabsModelsError"
        :voices-loading="elevenLabsVoicesLoading"
        :voices-error="elevenLabsVoicesError"
        :first-load-loading="elevenLabsFirstLoadLoading"
        @select="setActiveProvider('elevenlabs')"
        @toggle="toggleProvider('elevenlabs')"
        :on-save-key="saveElevenLabsApiKey"
        :on-apply-generation="saveElevenLabsGenerationSettings"
        @select-voice="selectElevenLabsVoice"
        @toggle-proxy="toggleElevenLabsUseProxy"
        @refresh-models="refreshElevenLabsModels"
        @refresh-voices="refreshElevenLabsVoices"
      />

      <!-- Local Provider -->
      <TtsLocalCard
        v-if="localVisible || activeProvider === 'local'"
        :active="activeProvider === 'local'"
        :expanded="providers.local.expanded"
        :url="localTtsUrl"
        @select="setActiveProvider('local')"
        @toggle="toggleProvider('local')"
        @save="saveLocalTtsUrl"
      />

      <!-- Pocket TTS local provider -->
      <TtsPocketCard
        v-show="visibleIds.includes(POCKET_VISIBILITY_ID)"
        :active-model-id="activePocketId"
        :speaker-device="speakerDevice"
        :speaker-volume="speakerVolume"
        @activate="handlePocketActivated"
      />

      <!-- Piper local provider -->
      <TtsPiperCard
        v-if="piperVisible"
        :providers="piperProviders"
        :active-id="activePiperId"
        :loading="piperLoading"
        :errors="piperError"
        @select="selectPiperProvider"
        @refreshed="applyPiperSnapshot"
      />
    </div>

    <!-- Telegram Auth Modal -->
    <TelegramAuthModal v-model="showTelegramModal" />
  </div>
</template>

<style scoped>
.tts-panel {
  max-width: 900px;
  margin: 0 auto;
}

.visibility-control {
  position: relative;
  top: -12px;
  display: flex;
  justify-content: flex-end;
  margin-bottom: -4px;
}

.visibility-popover {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 20;
  min-width: 260px;
  padding: 8px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.3));
  border-radius: 8px;
  background: var(--color-bg-elevated);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
}

.visibility-group + .visibility-group {
  margin-top: 8px;
}

.visibility-group-label {
  padding: 4px 8px;
  font-size: var(--ui-text-size-hint);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  color: var(--color-text-secondary, #888888);
}

.visibility-entry {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
}

.visibility-entry:hover {
  background: var(--color-background-hover, rgba(128, 128, 128, 0.12));
}

.visibility-entry input {
  margin: 0;
}

.visibility-entry-label {
  flex: 1;
}

.visibility-active {
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary, #888888);
}

.provider-cards {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

@media (max-width: 480px) {
  .visibility-popover {
    min-width: 0;
    width: max-content;
    max-width: calc(100vw - 24px);
  }
}
</style>
