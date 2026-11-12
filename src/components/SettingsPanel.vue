<script setup lang="ts">
import {
  nextTick,
  onBeforeUnmount,
  onMounted,
  onUpdated,
  ref,
  type Component,
} from 'vue'
import { Settings, Network, Type, Sparkles, PanelsTopLeft, Package } from 'lucide-vue-next'
import { t } from '../i18n'
import type { ModelCategory } from '../composables/useModelDownloads'
import SettingsGeneral from './settings/SettingsGeneral.vue'
import SettingsInterface from './settings/SettingsInterface.vue'
import SettingsEditor from './settings/SettingsEditor.vue'
import SettingsNetwork from './settings/SettingsNetwork.vue'
import SettingsModels from './settings/SettingsModels.vue'
import SettingsAiPanel from './SettingsAiPanel.vue'

type TabType = 'general' | 'interface' | 'editor' | 'network' | 'models' | 'ai'

interface TabDefinition {
  id: TabType
  labelKey: string
  icon: Component
}

const props = defineProps<{ active?: boolean }>()

const tabDefinitions: TabDefinition[] = [
  { id: 'general', labelKey: 'settings.tabs.general', icon: Settings },
  { id: 'interface', labelKey: 'settings.tabs.interface', icon: PanelsTopLeft },
  { id: 'editor', labelKey: 'settings.tabs.editor', icon: Type },
  { id: 'network', labelKey: 'settings.tabs.network', icon: Network },
  { id: 'models', labelKey: 'settings.tabs.models', icon: Package },
  { id: 'ai', labelKey: 'settings.tabs.ai', icon: Sparkles },
]

const activeTab = ref<TabType>('general')
const tabsElement = ref<HTMLElement | null>(null)
const modelsPanelRef = ref<{ focusCategory: (category: ModelCategory) => void } | null>(null)
// 1 is the safe initial layout while the panel is hidden: once the tablist has
// a measurable width, the observer chooses 6 (single row), 3 (preferred 3+3),
// 2, or 1 whole-tab columns.
const tabColumns = ref<1 | 2 | 3 | 6>(1)
let resizeObserver: ResizeObserver | null = null
let measureFrame: number | null = null

function tabNaturalWidth(button: HTMLButtonElement): number {
  const style = window.getComputedStyle(button)
  const icon = button.querySelector('svg')
  const label = button.querySelector('span')
  const iconWidth = icon?.getBoundingClientRect().width ?? 0
  const labelWidth = label?.scrollWidth ?? 0
  const gap = icon && label ? parseFloat(style.columnGap) || 0 : 0
  return (parseFloat(style.paddingLeft) || 0)
    + (parseFloat(style.paddingRight) || 0)
    + (parseFloat(style.borderLeftWidth) || 0)
    + (parseFloat(style.borderRightWidth) || 0)
    + iconWidth
    + labelWidth
    + gap
}

function measureTabs(): void {
  const container = tabsElement.value
  if (!container) return
  const available = container.clientWidth
  if (available <= 0) return

  const buttons = Array.from(container.querySelectorAll<HTMLButtonElement>('.ui-tab'))
  if (buttons.length === 0) return
  const widths = buttons.map(tabNaturalWidth)
  const total = widths.reduce((sum, width) => sum + width, 0)
  if (total <= available) {
    tabColumns.value = 6
    return
  }

  const widest = Math.max(...widths)
  if (widest * 3 <= available) tabColumns.value = 3
  else if (widest * 2 <= available) tabColumns.value = 2
  else tabColumns.value = 1
}

function scheduleMeasure(): void {
  if (typeof window === 'undefined') return
  if (typeof window.requestAnimationFrame !== 'function') {
    measureTabs()
    return
  }
  if (measureFrame !== null) return
  measureFrame = window.requestAnimationFrame(() => {
    measureFrame = null
    measureTabs()
  })
}

function handleWindowResize(): void {
  scheduleMeasure()
}

onMounted(() => {
  scheduleMeasure()
  if (typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(scheduleMeasure)
    if (tabsElement.value) {
      resizeObserver.observe(tabsElement.value)
      // Observe each tab as well: changing the interface font or locale can
      // change a tab's natural width without changing the tablist box.
      tabsElement.value.querySelectorAll('.ui-tab').forEach((tab) => {
        resizeObserver?.observe(tab)
      })
    }
  }
  window.addEventListener('resize', handleWindowResize)
  void document.fonts?.ready.then(scheduleMeasure).catch(() => {})
})

onUpdated(() => {
  scheduleMeasure()
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
  window.removeEventListener('resize', handleWindowResize)
  if (measureFrame !== null) {
    window.cancelAnimationFrame(measureFrame)
    measureFrame = null
  }
  if (errorTimeout !== null) {
    clearTimeout(errorTimeout)
    errorTimeout = null
  }
})

async function openModels(category: ModelCategory): Promise<void> {
  activeTab.value = 'models'
  await nextTick()
  modelsPanelRef.value?.focusCategory(category)
}

defineExpose({ openModels })

// Explicit notification severity. Legacy children that emit a message without a
// severity render as neutral info; severity is never inferred from translated
// substrings.
type MessageSeverity = 'error' | 'success' | 'warning' | 'info'
const messageSeverity = ref<MessageSeverity>('info')

// Error/Info Message Display
const errorMessage = ref<string | null>(null)
let errorTimeout: number | null = null

function showErrorMessage(message: string, severity: MessageSeverity = 'info') {
  errorMessage.value = message
  messageSeverity.value = severity

  if (errorTimeout !== null) {
    clearTimeout(errorTimeout)
  }

  errorTimeout = window.setTimeout(() => {
    errorMessage.value = null
    errorTimeout = null
  }, 3000)
}

function handleMessage(message: string, severity: MessageSeverity = 'info') {
  showErrorMessage(message, severity)
}
</script>

<template>
  <div class="settings-panel">
    <!-- Error/Info Message Display -->
    <div
      v-if="errorMessage"
      class="message-box ui-notice ui-notice--panel"
      :class="[messageSeverity, `ui-notice--${messageSeverity}`]"
    >
      {{ errorMessage }}
    </div>

    <!-- Tabs Navigation. The measured class chooses one row when all six tabs
         fit, otherwise the preferred General/Interface/Editor and
         Network/Models/AI rows, then whole-tab columns on narrower panels. -->
    <div
      ref="tabsElement"
      class="settings-tabs ui-tabs"
      :class="`settings-tabs--cols-${tabColumns}`"
      role="tablist"
    >
      <button
        v-for="tab in tabDefinitions"
        :key="tab.id"
        type="button"
        class="ui-tab"
        role="tab"
        :aria-selected="activeTab === tab.id"
        @click="activeTab = tab.id"
      >
        <component :is="tab.icon" :size="18" />
        <span>{{ t(tab.labelKey) }}</span>
      </button>
    </div>

    <!-- Tab Contents -->
    <SettingsGeneral
      v-show="activeTab === 'general'"
      @show-message="handleMessage"
    />
    <SettingsInterface
      v-show="activeTab === 'interface'"
      @show-message="handleMessage"
    />
    <SettingsEditor
      v-show="activeTab === 'editor'"
      @show-message="handleMessage"
    />
    <SettingsNetwork
      v-show="activeTab === 'network'"
      @show-message="handleMessage"
    />
    <SettingsModels
      v-show="activeTab === 'models'"
      ref="modelsPanelRef"
      :active="props.active === true && activeTab === 'models'"
      @show-message="handleMessage"
    />
    <SettingsAiPanel v-show="activeTab === 'ai'" />
  </div>
</template>

<style scoped>
.settings-panel {
  max-width: 900px;
  margin: 0 auto;
}

/* ============================================================================
 * Tabs
 * ============================================================================
 */

.settings-tabs {
  display: grid;
  justify-content: start;
  gap: 0;
  min-width: 0;
}

.settings-tabs--cols-6 {
  grid-template-columns: repeat(6, max-content);
}

.settings-tabs--cols-3 {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.settings-tabs--cols-2 {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.settings-tabs--cols-1 {
  grid-template-columns: minmax(0, 1fr);
}

.settings-tabs--cols-3 .ui-tab,
.settings-tabs--cols-2 .ui-tab,
.settings-tabs--cols-1 .ui-tab {
  justify-content: center;
}

.settings-tabs .ui-tab {
  min-width: 0;
}

.settings-tabs .ui-tab span {
  white-space: nowrap;
}

.tab-content {
  animation: fadeIn 0.2s ease;
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(-5px); }
  to { opacity: 1; transform: translateY(0); }
}
</style>
