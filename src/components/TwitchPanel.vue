<script setup lang="ts">
import { computed, ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { Eye, EyeOff, Play, Square, RotateCw, Check, Bot, Twitch, Loader2, X, Pencil, LogOut, Trash2, Plus, Copy, ExternalLink } from 'lucide-vue-next'
import { useTwitch } from '../composables/useTwitch'
import { useTwitchApiAuth } from '../composables/useTwitchApiAuth'
import { t } from '../i18n'
import PanelEasterEgg from './shared/PanelEasterEgg.vue'

const {
  settings,
  errorMessage,
  errorMessageType,
  currentStatus,
  connectionError,
  fieldErrors,
  showToken,
  isConnected,
  connectPending,
  modePending,
  restartTwitch,
  stopTwitch,
  startTwitch,
  save,
  saveStartOnBoot,
  saveSendOriginalText,
  saveMode,
  testMessage,
  isSendingTest,
  sendTestMessage,
  showError,
} = useTwitch()

const apiAuth = useTwitchApiAuth()

// Управление и реквизиты блокируются на всю операцию подключения, включая
// фоновое переподключение: правка в полёте гоняется с сохранением перед ним.
const connectionBusy = computed(
  () => connectPending.value || modePending.value || currentStatus.value === 'Connecting',
)

const apiStartBlockReason = computed(() => {
  if (settings.value.mode !== 'api') return ''
  if (!apiAuth.status.value || apiAuth.loading.value || apiAuth.authorizingRole.value) return t('twitch.api.start_wait')
  if (!apiAuth.isConfigured.value) return t('twitch.api.start_credentials')
  if (!apiAuth.botAccount.value) return t('twitch.api.start_bot')
  if (!apiAuth.selectedChannel.value) return t('twitch.api.select_channel_hint')
  return ''
})

const apiClientId = ref('')
const apiClientSecret = ref('')
const showClientId = ref(false)
const showClientSecret = ref(false)
const replacingSecret = ref(false)
const secretEntryVisible = computed(() => !apiAuth.status.value?.secretConfigured || replacingSecret.value)
const clientCredentialsBusy = computed(() => connectionBusy.value || apiAuth.loading.value || !!apiAuth.authorizingRole.value)

// Channel row mutation (select/remove) is blocked while any authorization,
// credential save or connection change is in flight.
const channelMutationBusy = computed(
  () => connectionBusy.value || apiAuth.loading.value || !!apiAuth.authorizingRole.value,
)

function cancelSecretReplacement() {
  apiClientSecret.value = ''
  showClientSecret.value = false
  replacingSecret.value = false
}
const copiedActivationLink = ref(false)
let copyActivationFeedbackTimer: ReturnType<typeof setTimeout> | undefined
let panelUnmounted = false

onMounted(() => {
  apiAuth.loadStatus()
})

// Any link change (including clearing) invalidates the previous session's
// feedback: drop the stale timer and reset the copied state so a new link can
// never inherit the previous "Copied" indicator.
watch(
  () => apiAuth.deviceVerificationUrl.value,
  () => {
    clearTimeout(copyActivationFeedbackTimer)
    copyActivationFeedbackTimer = undefined
    copiedActivationLink.value = false
  },
)

watch(
  () => apiAuth.status.value?.clientId,
  (id) => {
    if (id !== undefined) {
      apiClientId.value = id
    }
  },
  { immediate: true },
)

watch(
  () => settings.value.mode,
  (_newMode, oldMode) => {
    showClientId.value = false
    showClientSecret.value = false
    cancelSecretReplacement()
    if (oldMode === 'api' && apiAuth.authorizingRole.value) {
      apiAuth.cancelAuth()
    }
  },
)

onBeforeUnmount(() => {
  panelUnmounted = true
  clearTimeout(copyActivationFeedbackTimer)
  if (apiAuth.authorizingRole.value) {
    apiAuth.cancelAuth()
  }
})

async function copyActivationLink() {
  const url = apiAuth.deviceVerificationUrl.value
  const role = apiAuth.authorizingRole.value
  if (!url) return
  try {
    await navigator.clipboard.writeText(url)
    // The clipboard write is async: do not show feedback if the panel was
    // unmounted, the link changed, or the device session already ended.
    if (panelUnmounted || apiAuth.deviceVerificationUrl.value !== url
      || apiAuth.authorizingRole.value !== role) {
      return
    }
    copiedActivationLink.value = true
    showError(t('twitch.api.copied'), 'success')
    clearTimeout(copyActivationFeedbackTimer)
    copyActivationFeedbackTimer = setTimeout(() => {
      copiedActivationLink.value = false
    }, 2000)
  } catch (_) {
    if (panelUnmounted || apiAuth.deviceVerificationUrl.value !== url
      || apiAuth.authorizingRole.value !== role) return
    showError(t('twitch.api.copy_failed'), 'error')
  }
}

async function onOpenActivationLink() {
  await apiAuth.openActivationUrl()
}

async function onSelectChannel(userId: string) {
  if (channelMutationBusy.value) return
  await apiAuth.selectChannel(userId)
}

async function onRemoveChannel(userId: string) {
  if (channelMutationBusy.value) return
  await apiAuth.forgetChannel(userId)
}

async function onSaveClient() {
  const success = await apiAuth.saveClient(apiClientId.value, apiClientSecret.value)
  if (success) {
    cancelSecretReplacement()
    showError(t('twitch.api.credentials_saved'), 'success')
  }
}
</script>

<template>
  <div class="twitch-panel">
    <!-- Error/Info Message Display -->
    <div v-if="errorMessage" class="message-box ui-status" :class="errorMessageType">
      {{ errorMessage }}
    </div>

    <p v-if="connectionError" class="connection-error ui-status" role="alert">{{ connectionError }}</p>

    <section class="settings-section ui-section connection-section">
      <PanelEasterEgg kind="cassette" />
      <div class="section-header server-header">
        <h2 class="ui-section-title">{{ t('twitch.connection') }}</h2>
        <div class="server-status">
          <span class="status-indicator ui-status" :class="{
            running: currentStatus === 'Connected',
            connecting: currentStatus === 'Connecting',
            error: currentStatus === 'Error'
          }">
            {{ currentStatus === 'Connected' ? t('twitch.status.connected') :
               currentStatus === 'Connecting' ? t('twitch.status.connecting') :
               currentStatus === 'Error' ? t('twitch.status.error') :
               t('twitch.status.disconnected') }}
          </span>
          <template v-if="currentStatus === 'Connected'">
            <button @click="restartTwitch" class="status-button refresh ui-icon-button ui-icon-button--accent" :disabled="connectionBusy" :class="{ disabled: connectionBusy }" :title="t('twitch.restart')" :aria-label="t('twitch.restart')">
              <RotateCw :size="18" />
            </button>
            <button @click="stopTwitch" class="status-button stop ui-icon-button ui-action--stop" :disabled="connectPending" :class="{ disabled: connectPending }" :title="t('twitch.disconnect')" :aria-label="t('twitch.disconnect')">
              <Square :size="18" />
            </button>
          </template>
          <template v-else>
            <button @click="startTwitch" class="status-button start ui-icon-button ui-icon-button--accent" :disabled="connectionBusy || !!apiStartBlockReason" :class="{ disabled: connectionBusy || !!apiStartBlockReason }" :title="apiStartBlockReason || t('twitch.connect')" :aria-label="t('twitch.connect')" :aria-describedby="apiStartBlockReason ? 'twitch-api-start-reason' : undefined">
              <Play :size="18" />
            </button>
            <button @click="stopTwitch" class="status-button stop disabled ui-icon-button ui-action--stop" :title="t('twitch.disconnect')" :aria-label="t('twitch.disconnect')" disabled>
              <Square :size="18" />
            </button>
          </template>
        </div>
      </div>

      <p v-if="apiStartBlockReason && currentStatus !== 'Connected'" id="twitch-api-start-reason" class="ui-hint">{{ apiStartBlockReason }}</p>
      <div class="ui-row">
        <label class="ui-choice-label">
          <input type="checkbox" v-model="settings.start_on_boot" @change="saveStartOnBoot" :disabled="modePending" class="ui-choice-input" />
          <span>{{ t('twitch.start_on_boot') }}</span>
        </label>
      </div>

      <div class="ui-row">
        <label class="ui-choice-label">
          <input type="checkbox" v-model="settings.send_original_text" @change="saveSendOriginalText" :disabled="modePending" class="ui-choice-input" />
          <span>{{ t('twitch.send_original_text') }}</span>
        </label>
      </div>

      <!-- Divider after checkboxes -->
      <div class="subsection-divider"></div>

      <!-- Connection Mode Radio Switch -->
      <div class="ui-row mode-selector-row">
        <label class="ui-label">{{ t('twitch.mode') }}</label>
        <div class="mode-options">
          <label class="ui-choice-label setting-label radio-label" :class="{ 'is-selected': settings.mode === 'irc' }">
            <input
              type="radio"
              value="irc"
              name="twitch-mode"
              :checked="settings.mode === 'irc'"
              class="ui-choice-input"
              :disabled="connectionBusy"
              @change="saveMode('irc')"
            />
            <span>{{ t('twitch.mode.irc') }}</span>
          </label>
          <label class="ui-choice-label setting-label radio-label" :class="{ 'is-selected': settings.mode === 'api' }">
            <input
              type="radio"
              value="api"
              name="twitch-mode"
              :checked="settings.mode === 'api'"
              class="ui-choice-input"
              :disabled="connectionBusy"
              @change="saveMode('api')"
            />
            <span>{{ t('twitch.mode.api') }}</span>
          </label>
        </div>
      </div>

      <!-- IRC Credentials Grid (mode === 'irc') -->
      <div v-if="settings.mode === 'irc'" class="credentials-grid">
        <div class="identity-fields">
          <div class="ui-row credential-row">
            <label for="twitch-username" class="ui-label">{{ t('twitch.username') }}</label>
            <input
              type="text"
              v-model="settings.username"
              id="twitch-username"
              :aria-invalid="!!fieldErrors.username"
              :aria-describedby="fieldErrors.username ? 'twitch-username-feedback' : undefined"
              class="ui-input"
              :disabled="connectionBusy"
              placeholder="your_bot_username"
            />
          </div>
          <div class="ui-row credential-row">
            <label for="twitch-channel" class="ui-label">{{ t('twitch.channel') }}</label>
            <input
              type="text"
              v-model="settings.channel"
              id="twitch-channel"
              :aria-invalid="!!fieldErrors.channel"
              :aria-describedby="fieldErrors.channel ? 'twitch-channel-feedback' : undefined"
              class="ui-input"
              :disabled="connectionBusy"
              :placeholder="t('twitch.channel_placeholder')"
            />
          </div>
          <div v-if="fieldErrors.username || fieldErrors.channel" class="identity-feedback">
            <div v-if="fieldErrors.username" id="twitch-username-feedback" class="field-error ui-status" role="alert">{{ t('twitch.username') }}: {{ fieldErrors.username }}</div>
            <div v-if="fieldErrors.channel" id="twitch-channel-feedback" class="field-error ui-status" role="alert">{{ fieldErrors.channel }}</div>
          </div>
        </div>

        <div class="ui-row credential-row">
          <label for="twitch-token" class="ui-label">{{ t('twitch.token') }}</label>
          <div class="input-with-toggle">
            <input
              :type="showToken ? 'text' : 'password'"
              v-model="settings.token"
              id="twitch-token"
              :aria-invalid="!!fieldErrors.token"
              :aria-describedby="fieldErrors.token ? 'twitch-token-feedback' : undefined"
              class="ui-input"
              :disabled="connectionBusy"
              placeholder="xxxxxxxxxxxxxx"
            />
            <button
              type="button"
              class="toggle-icon-button ui-icon-button ui-icon-button--inset"
              @click="showToken = !showToken"
              :title="showToken ? t('twitch.token.hide') : t('twitch.token.show')"
              :aria-label="showToken ? t('twitch.token.hide') : t('twitch.token.show')"
            >
              <Eye v-if="!showToken" :size="18" />
              <EyeOff v-else :size="18" />
            </button>
          </div>
          <small v-if="fieldErrors.token" id="twitch-token-feedback" class="field-feedback field-error" role="alert">{{ fieldErrors.token }}</small>
        </div>

        <div class="ui-row button-row">
          <button @click="save" class="save-button-inline ui-button ui-button--primary" :disabled="connectionBusy" :class="{ disabled: connectionBusy }">{{ t('common.save') }}</button>
        </div>
      </div>

      <!-- Twitch API Section (mode === 'api') -->
      <div v-else class="api-credentials-section">
        <p v-if="apiAuth.errorMessage.value || apiAuth.storeErrorMessage.value" class="connection-error ui-status" role="alert">
          {{ apiAuth.errorMessage.value || apiAuth.storeErrorMessage.value }}
        </p>

        <!-- Credentials Form -->
        <div class="api-credentials-grid">
          <!-- Client ID -->
          <div class="ui-row credential-row">
            <label for="twitch-client-id" class="ui-label">{{ t('twitch.api.client_id') }}</label>
            <div class="input-with-toggle">
            <input
              :type="showClientId ? 'text' : 'password'"
              v-model="apiClientId"
              id="twitch-client-id"
              class="ui-input"
              :disabled="clientCredentialsBusy"
              placeholder="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
            />
              <button type="button" class="toggle-icon-button ui-icon-button ui-icon-button--inset"
                @click="showClientId = !showClientId"
                :title="showClientId ? t('twitch.token.hide') : t('twitch.token.show')"
                :aria-label="showClientId ? t('twitch.token.hide') : t('twitch.token.show')"
                :aria-pressed="showClientId">
                <Eye v-if="!showClientId" :size="18" />
                <EyeOff v-else :size="18" />
              </button>
            </div>
          </div>

          <!-- Client Secret -->
          <div class="ui-row credential-row">
            <label :for="secretEntryVisible ? 'twitch-client-secret' : undefined" class="ui-label">{{ t('twitch.api.client_secret') }}</label>
            <div class="secret-field-wrapper">
              <template v-if="!secretEntryVisible">
                <div class="secret-saved-row">
                  <span class="ui-status"><Check :size="16" aria-hidden="true" /> {{ t('twitch.api.client_secret_set') }}</span>
                  <button type="button" class="ui-icon-button" :disabled="clientCredentialsBusy" @click="replacingSecret = true"
                    :title="t('twitch.api.client_secret_replace')" :aria-label="t('twitch.api.client_secret_replace')">
                    <Pencil :size="18" aria-hidden="true" />
                  </button>
                </div>
              </template>
              <template v-else>
              <div class="secret-entry-row">
              <div class="input-with-toggle">
              <input
                :type="showClientSecret ? 'text' : 'password'"
                v-model="apiClientSecret"
                id="twitch-client-secret"
                class="ui-input"
                :disabled="clientCredentialsBusy"
                :placeholder="t('twitch.api.client_secret_placeholder')"
              />
                <button type="button" class="toggle-icon-button ui-icon-button ui-icon-button--inset"
                  @click="showClientSecret = !showClientSecret"
                  :title="showClientSecret ? t('twitch.token.hide') : t('twitch.token.show')"
                  :aria-label="showClientSecret ? t('twitch.token.hide') : t('twitch.token.show')"
                  :aria-pressed="showClientSecret">
                  <Eye v-if="!showClientSecret" :size="18" />
                  <EyeOff v-else :size="18" />
                </button>
              </div>
              <button v-if="replacingSecret" type="button" class="ui-icon-button ui-icon-button--adjacent secret-cancel" :disabled="clientCredentialsBusy" @click="cancelSecretReplacement"
                :title="t('common.cancel')" :aria-label="t('common.cancel')">
                <X :size="18" aria-hidden="true" />
              </button>
              </div>
              </template>
            </div>
          </div>

          <!-- Save API Credentials Button -->
          <div class="ui-row button-row">
            <button
              type="button"
              class="ui-button ui-button--primary save-button-inline"
              :disabled="clientCredentialsBusy || !apiClientId.trim() || (secretEntryVisible && !apiClientSecret.trim())"
              @click="onSaveClient"
            >
              {{ t('common.save') }}
            </button>
          </div>
        </div>

      </div>
    </section>

    <section v-if="settings.mode === 'api'" class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('twitch.api.accounts_title') }}</h2>
      <div class="api-accounts-section">
        <p class="api-account-note ui-hint">
          {{ t('twitch.api.account_note') }}
        </p>

        <!-- Account Authorizations (Stacked Vertically) -->
        <div class="api-accounts-stack">
          <!-- Bot Account Card -->
          <div class="account-card" :class="{ 'is-authorized': !!apiAuth.botAccount.value }">
            <div class="account-card-header">
              <Bot :size="20" class="account-icon" />
              <div class="account-title-group">
                <span class="account-title ui-group-title">{{ t('twitch.api.bot_account') }}</span>
                <span class="ui-hint">{{ t('twitch.api.bot_account_hint') }}</span>
              </div>
            </div>
            <div class="account-status-row" :class="{ 'account-status-row--empty': !apiAuth.botAccount.value }">
              <span v-if="apiAuth.botAccount.value" class="bot-auth-status ui-status">
                <span class="bot-auth-login">@{{ apiAuth.botAccount.value.login }}</span>
              </span>
              <div class="account-action-buttons">
                <template v-if="apiAuth.authorizingRole.value === 'bot'">
                  <span class="authorizing-spinner ui-status">
                    <Loader2 :size="16" class="spin-icon" />
                    {{ t('twitch.api.activation_pending') }}
                  </span>
                  <button type="button" class="ui-button ui-button--secondary" @click="apiAuth.cancelAuth">
                    {{ t('twitch.api.cancel_auth') }}
                  </button>
                </template>
                <template v-else>
                  <button v-if="apiAuth.botAccount.value" type="button" class="ui-icon-button ui-action--danger bot-reset"
                    :title="t('twitch.api.reset_bot')" :aria-label="t('twitch.api.reset_bot')"
                    :disabled="clientCredentialsBusy" @click="apiAuth.clearAuth()">
                    <LogOut :size="18" aria-hidden="true" />
                  </button>
                  <button
                    v-else
                    type="button"
                    class="ui-button ui-button--secondary"
                    :disabled="clientCredentialsBusy || !apiAuth.isConfigured.value"
                    @click="apiAuth.beginAuth('bot')"
                  >
                    {{ t('twitch.api.authorize') }}
                  </button>
                </template>
              </div>
            </div>
            <div v-if="apiAuth.authorizingRole.value === 'bot'" class="bot-device-authorization device-auth-block">
                <p class="device-help ui-description">
                  {{ t('twitch.api.bot_device_help') }}
                </p>
                <div v-if="apiAuth.deviceVerificationUrl.value" class="device-verification">
                  <div class="device-verification-actions ui-composite">
                    <input class="ui-input ui-composite-field device-verification-uri" type="text" readonly :value="apiAuth.deviceVerificationUrl.value" :aria-label="t('twitch.api.device_link_label')" :title="apiAuth.deviceVerificationUrl.value" />
                    <button type="button" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" @click="copyActivationLink"
                      :title="copiedActivationLink ? t('twitch.api.copied') : t('twitch.api.copy_link')"
                      :aria-label="copiedActivationLink ? t('twitch.api.copied') : t('twitch.api.copy_link')">
                      <Check v-if="copiedActivationLink" :size="18" aria-hidden="true" /><Copy v-else :size="18" aria-hidden="true" />
                    </button>
                    <button type="button" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" @click="onOpenActivationLink"
                      :title="t('twitch.api.open_link')" :aria-label="t('twitch.api.open_link')">
                      <ExternalLink :size="18" aria-hidden="true" />
                    </button>
                  </div>
                </div>
            </div>
          </div>

          <!-- Channels Card Container -->
          <div class="account-card channels-container" :class="{ 'is-authorized': apiAuth.channels.value.length > 0 }">
            <div class="account-card-header">
              <Twitch :size="20" class="account-icon" />
              <div class="account-title-group">
                <span class="account-title ui-group-title">{{ t('twitch.api.channels') }}</span>
                <span class="ui-hint">{{ t('twitch.api.channels_hint') }}</span>
              </div>
              <button v-if="apiAuth.authorizingRole.value !== 'broadcaster'" type="button"
                class="ui-icon-button add-channel-button"
                :disabled="clientCredentialsBusy || !apiAuth.isConfigured.value"
                :title="t('twitch.api.add_channel')" :aria-label="t('twitch.api.add_channel')"
                @click="apiAuth.beginDeviceAuth()">
                <Plus :size="18" aria-hidden="true" />
              </button>
            </div>
            <div class="channel-list">
              <p v-if="apiAuth.channels.value.length === 0" class="channel-empty ui-description">
                {{ t('twitch.api.channels_empty') }}
              </p>
              <template v-else>
                <div
                  v-for="channel in apiAuth.channels.value"
                  :key="channel.userId"
                  class="channel-row"
                  :class="{ 'is-selected': channel.userId === apiAuth.selectedChannelId.value }"
                >
                  <label class="ui-choice-label channel-recipient">
                    <input
                      type="radio"
                      class="ui-choice-input"
                      name="twitch-channel-recipient"
                      :value="channel.userId"
                      :checked="channel.userId === apiAuth.selectedChannelId.value"
                      :disabled="channelMutationBusy"
                      @change="onSelectChannel(channel.userId)"
                    />
                    <span class="channel-login">{{ channel.login }}</span>
                  </label>
                  <button
                    type="button"
                    class="ui-icon-button ui-action--danger channel-remove"
                    :disabled="channelMutationBusy"
                    :title="t('twitch.api.remove_access_for', { login: channel.login })"
                    :aria-label="t('twitch.api.remove_access_for', { login: channel.login })"
                    @click="onRemoveChannel(channel.userId)"
                  >
                    <Trash2 :size="18" aria-hidden="true" />
                  </button>
                </div>
                <p v-if="apiAuth.selectedChannelId.value === null" class="channel-hint ui-hint">
                  {{ t('twitch.api.select_channel_hint') }}
                </p>
              </template>
            </div>
            <div v-if="apiAuth.authorizingRole.value === 'broadcaster'" class="channel-add device-auth-block">
              <div class="channel-add-waiting">
                <span class="authorizing-spinner ui-status">
                  <Loader2 :size="16" class="spin-icon" />
                  {{ t('twitch.api.activation_pending') }}
                </span>
                <button type="button" class="ui-button ui-button--secondary" @click="apiAuth.cancelAuth">
                  {{ t('twitch.api.cancel_auth') }}
                </button>
              </div>
              <p class="device-help ui-description">
                {{ t('twitch.api.device_help') }}
              </p>
              <div v-if="apiAuth.deviceVerificationUrl.value" class="device-verification">
                <div class="device-verification-actions ui-composite">
                  <input class="ui-input ui-composite-field device-verification-uri" type="text" readonly :value="apiAuth.deviceVerificationUrl.value" :aria-label="t('twitch.api.device_link_label')" :title="apiAuth.deviceVerificationUrl.value" />
                  <button type="button" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" @click="copyActivationLink"
                    :title="copiedActivationLink ? t('twitch.api.copied') : t('twitch.api.copy_link')"
                    :aria-label="copiedActivationLink ? t('twitch.api.copied') : t('twitch.api.copy_link')">
                    <Check v-if="copiedActivationLink" :size="18" aria-hidden="true" /><Copy v-else :size="18" aria-hidden="true" />
                  </button>
                  <button type="button" class="ui-icon-button ui-icon-button--adjacent ui-composite-action" @click="onOpenActivationLink"
                    :title="t('twitch.api.open_link')" :aria-label="t('twitch.api.open_link')">
                    <ExternalLink :size="18" aria-hidden="true" />
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>

      </div>
    </section>

    <section class="settings-section ui-section">
      <h2 class="ui-section-title">{{ t('twitch.test.title') }}</h2>
      <div class="ui-row">
        <input
          type="text"
          v-model="testMessage"
          :placeholder="t('twitch.test.placeholder')"
          class="ui-input test-input"
          @keyup.enter="sendTestMessage"
        />
        <button
          @click="sendTestMessage"
          class="test-button ui-button ui-button--primary"
          :disabled="connectionBusy || !isConnected || !testMessage.trim() || isSendingTest"
        >{{ isSendingTest ? t('twitch.test.sending') : t('twitch.test.send') }}</button>
      </div>
    </section>

    <section class="settings-section help-section ui-section">
      <h2 class="ui-section-title">{{ t('twitch.help.title') }}</h2>
      <template v-if="settings.mode === 'irc'">
        <p class="help-text ui-description">
          {{ t('twitch.help.oauth_intro') }}
        </p>
        <a href="https://twitchtokengenerator.com" target="_blank" rel="noopener noreferrer" class="help-link ui-description">
          https://twitchtokengenerator.com
        </a>
        <p class="help-text ui-description">
          {{ t('twitch.help.token_format_prefix') }}<code>oauth:</code>{{ t('twitch.help.token_format_suffix') }}
        </p>
      </template>
      <template v-else>
        <p class="help-text ui-description">
          {{ t('twitch.api.help_intro') }}
        </p>
        <a href="https://dev.twitch.tv/console/apps" target="_blank" rel="noopener noreferrer" class="help-link ui-description">
          {{ t('twitch.api.help_dev_console') }}
        </a>
        <p class="help-text ui-description">
          {{ t('twitch.api.help_step2') }}
        </p>
        <p class="help-text ui-description">
          {{ t('twitch.api.help_step4') }}
        </p>
        <p class="help-text ui-description">
          {{ t('twitch.api.remove_access_help') }}
        </p>
        <details class="help-registration ui-description">
          <summary>{{ t('twitch.api.help_registration') }}</summary>
          <p class="help-text">{{ t('twitch.api.help_step1') }}</p>
        </details>
      </template>
    </section>
  </div>
</template>

<style scoped>
.connection-section {
  position: relative;
}

.twitch-panel {
  max-width: 900px;
  margin: 0 auto;
}

h2 {
  margin-top: 0;
  margin-bottom: 1rem;
  color: var(--color-text-primary);
}

/* Section header */
.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

/* Server header with status */
.server-header {
  padding-top: 0;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--color-border);
  margin-bottom: 1rem;
  align-items: flex-start;
}

.server-header h2 {
  margin: 0;
}

.server-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: -2px;
}

/* Connection badge: local colors and compact surface, shared typography. */
.status-indicator {
  padding: 0.15rem 0.5rem;
  background: var(--color-bg-field);
  border-radius: 5px;
  border: 1px solid var(--color-border);
  height: 28px;
  display: flex;
  align-items: center;
}

.status-indicator.running {
  color: var(--success-text-bright);
  background: var(--success-bg-weak);
  border-color: var(--success-shadow);
}

.status-indicator.connecting {
  color: var(--warning-text-bright);
  background: var(--warning-bg-weak);
  border-color: var(--warning-border);
}

.status-indicator.error {
  color: var(--danger-text-weak);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

.status-button.disabled:not(.stop) {
  background: var(--btn-disabled-bg);
  cursor: not-allowed;
  opacity: 0.6;
}

.message-box {
  position: fixed;
  top: 20px;
  left: calc(50% + 100px);
  transform: translateX(-50%);
  padding: 0.4rem 0.75rem;
  border-radius: 8px;
  z-index: 1000;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
  backdrop-filter: blur(10px);
  animation: slideDownFade 0.3s ease-out;
  white-space: nowrap;
}

.message-box.success {
  background: var(--success-bg);
  border: 1px solid var(--success-border, rgba(74, 222, 128, 0.4));
  color: var(--success-text);
}

.message-box.error {
  background: var(--danger-bg);
  border: 1px solid var(--danger-border);
  border-left: 4px solid var(--status-disconnected);
  color: var(--danger-text);
}

.message-box.info {
  background: var(--info-bg);
  border: 1px solid var(--info-border);
  color: var(--info-text);
}

@keyframes slideDownFade {
  from {
    opacity: 0;
    transform: translateX(-50%) translateY(-20px);
  }
  to {
    opacity: 1;
    transform: translateX(-50%) translateY(0);
  }
}

/* Decorative card surface stays local; padding and rhythm come from ui-section. */
.settings-section {
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

/* Rows use ui-row; only wrap behavior and the last-row reset stay local.
   The legacy .setting-row name is gone so global AudioPanel rules cannot
   reach this panel. */
.ui-row {
  flex-wrap: wrap;
}

.ui-row:last-child {
  margin-bottom: 0;
}

.ui-row label {
  color: var(--color-text-secondary);
}

.connection-section {
  container-type: inline-size;
  container-name: twitch-connection;
}

.subsection-divider {
  border-top: 1px solid var(--color-border);
  margin: 1rem 0;
}

/* Mode selector */
.mode-selector-row {
  display: flex;
  align-items: center;
  gap: 1.5rem;
  margin-top: 0;
  padding-bottom: 0.75rem;
  border-bottom: 1px solid var(--color-border);
}

.mode-options {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.radio-label {
  padding: 0.35rem 0.75rem;
  border-radius: var(--ui-radius-sm, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-base);
  transition: border-color 0.15s ease, background-color 0.15s ease;
}

.radio-label.is-selected {
  border-color: var(--color-accent);
  background: var(--color-bg-field);
  color: var(--color-text-primary);
}

/* Labels use their text width; related identity errors span both fields. */
.identity-fields {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ui-settings-group-gap);
  margin-bottom: var(--ui-row-gap);
}

.credentials-grid {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr) max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
}

.credentials-grid .identity-fields {
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
  column-gap: var(--ui-row-label-gap-side);
}

.credentials-grid .identity-fields .credential-row {
  grid-column: span 2;
  grid-template-columns: subgrid;
}

.credentials-grid > .credential-row {
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
}

.credentials-grid > .button-row {
  grid-column: 1 / -1;
}

.credentials-grid > .credential-row > .input-with-toggle,
.credentials-grid > .credential-row > .field-feedback {
  grid-column: 2 / -1;
}

.identity-feedback {
  grid-column: 1 / -1;
  margin-top: calc(var(--ui-hint-gap) - var(--ui-settings-group-gap));
}

.identity-fields .credential-row {
  margin-bottom: 0;
  align-content: start;
}

.credential-row {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-hint-gap);
}

.credential-row > .ui-label {
  overflow-wrap: anywhere;
}

.credential-row > .field-feedback {
  grid-column: 2;
}

/* Twitch API specific styles */
.api-credentials-section,
.api-accounts-section {
  display: flex;
  flex-direction: column;
  gap: var(--ui-row-gap);
}

.api-accounts-section {
  gap: 8px;
}

.api-credentials-grid {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  column-gap: var(--ui-row-label-gap-side);
  row-gap: var(--ui-row-gap);
}

.api-credentials-grid > .credential-row {
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
  margin-bottom: 0;
}

.api-credentials-grid > .credential-row > .ui-label {
  align-self: center;
}

.api-credentials-grid > .credential-row > .ui-input {
  width: 100%;
  max-width: none;
}

.api-credentials-grid > .ui-row.button-row {
  grid-column: 1 / -1;
  margin: 0;
  margin-top: calc(8px - var(--ui-row-gap));
  padding-top: 8px;
}

.api-intro {
  margin: 0;
}

.secret-field-wrapper {
  min-width: 0;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}

.api-credentials-grid .secret-field-wrapper .ui-input {
  max-width: none;
  width: 100%;
}

.secret-saved-row {
  display: flex;
  align-items: center;
  gap: var(--ui-row-label-gap-side);
  flex-wrap: wrap;
  justify-content: flex-start;
  font-size: var(--ui-text-size-control);
  min-height: max(var(--ui-control-min-height), calc(1em * var(--ui-line-height-control) + 2 * var(--ui-control-padding-y) + 2 * var(--ui-border-width)));
}

.secret-saved-row .ui-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text-primary);
  font-size: var(--ui-text-size-control);
  font-weight: var(--ui-text-weight-control);
}

.secret-entry-row {
  display: flex;
  align-items: center;
  gap: var(--ui-row-label-gap-side);
  min-width: 0;
}

.api-accounts-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
}

.account-card {
  padding: 6px 4px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.account-card + .account-card {
  border-top: 1px solid var(--color-border);
  padding-top: 8px;
}

.account-card:last-child {
  padding-bottom: 0;
}

.account-card-header {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.account-icon {
  margin-top: 2px;
  color: var(--color-text-secondary);
}

.account-title-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.channels-container .account-title-group {
  flex: 1;
  min-width: 0;
}

.account-title {
  font-weight: var(--ui-text-weight-title);
  color: var(--color-text-primary);
}

.channel-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px;
  border: 1px solid var(--color-border);
  border-radius: 10px;
  background: var(--color-bg-field);
}

.channel-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
  padding: 4px;
  border-radius: 6px;
}

.channel-recipient {
  --ui-choice-size: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
  flex: 1;
}

.channel-login {
  color: var(--color-text-primary);
  overflow-wrap: anywhere;
}

.channel-remove {
  flex-shrink: 0;
}

.channel-empty {
  margin: 0;
  padding: 8px;
}

.channel-hint {
  margin: 0;
  overflow-wrap: anywhere;
}

.channel-add {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.channel-add-waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.device-verification-actions {
  width: 100%;
}

.channel-timeout {
  margin: 0;
  overflow-wrap: anywhere;
}

.bot-auth-status {
  display: inline-flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.bot-auth-login {
  color: var(--color-text-primary);
  font-size: var(--ui-text-size-control);
  font-weight: var(--ui-text-weight-control);
}

.account-status-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 6px 12px;
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: var(--ui-radius);
}

.account-action-buttons {
  display: flex;
  align-items: center;
  gap: 8px;
}

.authorizing-spinner {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--warning-text-bright);
}

.spin-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.api-account-note {
  margin: 0;
  line-height: 1.4;
}

/* Use the available form width, which excludes the sidebar and panel padding. */
@container twitch-connection (max-width: 480px) {
  .credentials-grid,
  .api-credentials-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .credentials-grid .identity-fields .credential-row {
    grid-column: 1;
  }

  .credentials-grid > .credential-row > .input-with-toggle,
  .credentials-grid > .credential-row > .field-feedback {
    grid-column: 1;
  }
  .identity-fields {
    grid-template-columns: minmax(0, 1fr);
  }

  .credential-row {
    grid-template-columns: minmax(0, 1fr);
    row-gap: var(--ui-row-label-gap-stack);
  }

  .api-credentials-grid > .credential-row > .ui-label {
    padding-top: 0;
  }

  .credential-row > .field-feedback {
    grid-column: 1;
    margin-top: calc(var(--ui-hint-gap) - var(--ui-row-label-gap-stack));
  }
}

.ui-row.button-row {
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--color-border);
}

/* Text actions keep their local accent surfaces; typography and geometry
   come from ui-button. */
/* Field width intent: inputs fill the row but stay readable on wide panels. */
.ui-row .ui-input {
  flex: 1;
  max-width: 400px;
}

/* Input with toggle icon button */
.input-with-toggle {
  position: relative;
  flex: 1;
  min-width: 0;
}

.input-with-toggle .ui-input::-ms-reveal {
  display: none;
}

.input-with-toggle .ui-input {
  flex: 1 1 auto;
  max-width: none;
  width: 100%;
  padding-right: 40px; /* Space for the inset button */
}

/* Local positioning of the inset eye button; geometry/transparency come from
   ui-icon-button--inset and it stays centered while the input grows. */
.toggle-icon-button {
  position: absolute;
  right: 4px;
  top: 50%;
  transform: translateY(-50%);
}

.help-text {
  margin: 0.5rem 0;
}

.help-section > :last-child,
.help-registration > :last-child {
  margin-bottom: 0;
}

.help-link {
  color: var(--color-info);
  text-decoration: none;
}

.help-link:hover {
  text-decoration: underline;
}

.help-text code {
  background: var(--info-bg-weak);
  padding: 0.2rem 0.4rem;
  border-radius: 4px;
  font-family: var(--font-mono);
  color: var(--color-info);
  border: 1px solid var(--info-border);
}

.activation-url {
  font-family: var(--font-mono);
  font-size: var(--ui-text-size-field);
  color: var(--color-info);
  word-break: break-all;
}

.twitch-panel {
  --twitch-error-text: var(--danger-text-bright);
}

:global([data-theme="light"]) .twitch-panel {
  --twitch-error-text: #b91c1c;
}

.connection-error {
  margin-bottom: 12px;
  padding: 12px;
  border: 1px solid var(--danger-border-strong);
  border-radius: 8px;
  background: var(--danger-bg-weak);
  color: var(--twitch-error-text);
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.message-box.error {
  background: var(--danger-bg-weak);
  border-color: var(--danger-border-strong);
  color: var(--twitch-error-text);
}

.field-feedback {
  color: var(--color-text-secondary);
  display: flex;
  flex-direction: column;
  gap: var(--ui-hint-gap);
  overflow-wrap: anywhere;
}

/* Hint/error texts under fields use the agreed 0.85rem/400 role values. */
.field-feedback,
.field-feedback small {
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  line-height: 1.4;
}

.field-error {
  color: var(--twitch-error-text);
}

.device-help {
  margin: 8px 0 0;
  overflow-wrap: anywhere;
}

.remove-access-help {
  margin: 0;
  overflow-wrap: anywhere;
}

.device-verification {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 6px;
}

.device-verification-label {
  margin: 0;
}

.device-verification-uri {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
}

.device-verification-uri.ui-input:hover:not(:disabled) {
  background: var(--color-bg-field);
}

.device-auth-block {
  padding: 12px;
  border: 1px solid var(--color-border);
  border-radius: var(--ui-radius);
  background: var(--color-bg-field);
}
.help-registration { margin: 8px 0; overflow-wrap: anywhere; }
.help-registration summary { cursor: pointer; }
.account-status-row--empty {
  justify-content: center;
}
</style>
