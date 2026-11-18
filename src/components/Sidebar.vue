<script setup lang="ts">
import { onMounted, ref, watch, h, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { debugError } from '../utils/debug'
import { t } from '../i18n'
import {
  Volume2,
  Speech,
  Globe,
  ChevronLeft,
  ChevronRight,
  LogOut,
  ClipboardPenLine,
  Pencil,
  Settings,
  Keyboard,
  Crosshair,
  Tv,
  Inbox,
  ScanText,
  MessageSquareText
} from 'lucide-vue-next'

// Custom Twitch icon component
const TwitchIcon = (props: { size?: number }) => h('svg', {
  xmlns: 'http://www.w3.org/2000/svg',
  width: props.size || 20,
  height: props.size || 20,
  viewBox: '0 0 24 24',
  fill: 'currentColor'
}, [
  h('path', { d: 'M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z' })
])

type Panel = 'input' | 'tts' | 'audio' | 'preprocessor' | 'webview' | 'twitch' | 'input-server' | 'vtube-studio' | 'vrchat' | 'ocr' | 'settings' | 'hotkeys' | 'intercept'

interface SidebarButton {
  id: Panel
  label: string
  icon: any
}

interface SidebarGroup {
  buttons: SidebarButton[]
}

const props = defineProps<{
  currentPanel: Panel
}>()

const emit = defineEmits<{
  setPanel: [panel: Panel]
}>()

function setPanel(panel: Panel) {
  emit('setPanel', panel)
}

async function quitApp() {
  try {
    await invoke('quit_app')
  } catch (e) {
    debugError('Failed to quit:', e)
  }
}

// Collapse state with localStorage persistence
const STORAGE_KEY = 'sidebar-collapsed'
const isCollapsed = ref(false)

onMounted(() => {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved !== null) {
    isCollapsed.value = saved === 'true'
  }
})

watch(isCollapsed, (newValue) => {
  localStorage.setItem(STORAGE_KEY, String(newValue))
})

// Sidebar groups structure (labels react to the active locale)
const sidebarGroups = computed<SidebarGroup[]>(() => [
  {
    buttons: [
      { id: 'input', label: t('nav.input'), icon: Pencil },
      { id: 'tts', label: t('nav.tts'), icon: Speech },
      { id: 'audio', label: t('nav.audio'), icon: Volume2 }
    ]
  },
  {
    buttons: [
      { id: 'webview', label: t('nav.webview'), icon: Globe },
      { id: 'twitch', label: t('nav.twitch'), icon: TwitchIcon },
      { id: 'vtube-studio', label: t('nav.vtube-studio'), icon: Tv },
      { id: 'vrchat', label: t('nav.vrchat'), icon: MessageSquareText },
      { id: 'ocr', label: t('nav.ocr'), icon: ScanText },
      { id: 'input-server', label: t('nav.input-server'), icon: Inbox }
    ]
  },
  {
    buttons: [
      { id: 'preprocessor', label: t('nav.preprocessor'), icon: ClipboardPenLine },
      { id: 'hotkeys', label: t('nav.hotkeys'), icon: Keyboard },
      { id: 'intercept', label: t('nav.intercept'), icon: Crosshair },
      { id: 'settings', label: t('nav.settings'), icon: Settings }
    ]
  }
])

function toggleCollapse() {
  isCollapsed.value = !isCollapsed.value
}
</script>

<template>
  <aside
    class="sidebar"
    :class="{ 'sidebar-collapsed': isCollapsed }"
  >
    <!-- Floating collapse button positioned outside sidebar -->
    <button
      class="collapse-toggle-floating"
      @click="toggleCollapse"
      :title="isCollapsed ? t('shell.sidebar.expand') : t('shell.sidebar.collapse')"
      :aria-label="isCollapsed ? t('shell.sidebar.expand') : t('shell.sidebar.collapse')"
    >
      <ChevronLeft v-if="!isCollapsed" :size="18" />
      <ChevronRight v-else :size="18" />
    </button>

    <nav class="sidebar-nav">
      <template v-for="(group, groupIndex) in sidebarGroups" :key="groupIndex">
        <div
          v-for="button in group.buttons"
          :key="button.id"
          class="sidebar-button-wrapper"
        >
          <button
            class="sidebar-button"
            :class="{ 'sidebar-button-active': props.currentPanel === button.id }"
            @click="setPanel(button.id)"
            :title="isCollapsed ? button.label : undefined"
            :aria-label="button.label"
          >
            <component :is="button.icon" :size="20" class="sidebar-icon" />
            <span v-if="!isCollapsed" class="sidebar-button-label">{{ button.label }}</span>
            <div v-if="props.currentPanel === button.id" class="active-indicator" />
          </button>
        </div>

        <div v-if="groupIndex < sidebarGroups.length - 1" class="sidebar-divider" />
      </template>
    </nav>

    <div class="sidebar-footer">
      <button
        class="sidebar-button quit-button"
        @click="quitApp"
        :title="isCollapsed ? t('nav.quit') : undefined"
        :aria-label="t('nav.quit')"
      >
        <LogOut :size="20" class="sidebar-icon" />
        <span v-if="!isCollapsed" class="sidebar-button-label">{{ t('nav.quit') }}</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  flex: 0 0 200px;
  width: 200px;
  min-width: 200px;
  position: relative;
  overflow: hidden;
  background:
    linear-gradient(180deg, rgba(var(--rgb-contrast), 0.03), transparent 22%),
    linear-gradient(
      180deg,
      rgba(var(--main-window-rgb), var(--main-window-opacity)) 0%,
      rgba(var(--main-window-rgb), var(--main-window-opacity)) 100%
    );
  color: var(--color-text-primary);
  display: flex;
  flex-direction: column;
  transition: width 0.28s ease, min-width 0.28s ease;
  box-shadow: inset -1px 0 0 var(--color-border-weak);
}

.sidebar::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(circle at top left, var(--color-accent-glow-strong), transparent 30%),
    linear-gradient(var(--grid-line-color) 1px, transparent 1px),
    linear-gradient(90deg, var(--grid-line-color) 1px, transparent 1px);
  background-size: auto, 18px 18px, 18px 18px;
  mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 0.95) 0%, rgba(0, 0, 0, 0.7) 78%, rgba(0, 0, 0, 0.92) 100%);
}

.sidebar-collapsed {
  flex-basis: 70px;
  width: 70px;
  min-width: 70px;
}

/* Floating collapse button positioned on right edge of sidebar */
.collapse-toggle-floating {
  position: absolute;
  right: -17px;
  top: calc(70% + 66px);
  transform: translateY(-50%);
  width: 34px;
  height: 34px;
  border: 1px solid var(--color-border-strong);
  background-color: transparent;
  background-image: linear-gradient(135deg, var(--color-bg-handle-top), var(--color-bg-handle-bottom));
  color: var(--color-text-secondary);
  cursor: pointer;
  padding: 0;
  border-radius: 999px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.25s ease;
  z-index: 1000;
  box-shadow:
    0 4px 16px rgba(0, 0, 0, 0.2),
    0 0 0 1px rgba(var(--rgb-contrast), 0.04),
    inset 0 1px 0 rgba(var(--rgb-contrast), 0.08);
}

.collapse-toggle-floating:hover {
  color: var(--color-text-primary);
  background-color: var(--sidebar-btn-hover-bg);
  background-image: none;
  border-color: var(--card-active-border);
  box-shadow:
    0 6px 24px rgba(0, 0, 0, 0.3),
    0 0 0 1px var(--color-accent-glow-strong),
    0 0 20px var(--color-accent-glow-strong),
    inset 0 1px 0 rgba(var(--rgb-contrast), 0.12);
  transform: translateY(-50%) scale(1.06);
}

.sidebar-collapsed .collapse-toggle-floating {
  right: -17px;
}

.collapse-toggle-floating svg {
  transform: translateX(-5px);
}

.sidebar-nav {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  flex: 1;
  overflow-y: auto;
  padding: 1rem 0 0.75rem;
}

.sidebar-button-wrapper {
  position: relative;
  margin-bottom: 0;
}

.sidebar-button {
  width: 100%;
  min-height: 30px;
  padding: 0 0.85rem 0 1rem;
  border: 1px solid transparent;
  background: var(--sidebar-btn-bg);
  color: var(--color-text-secondary);
  cursor: pointer;
  text-align: left;
  transition: all 0.18s ease;
  display: flex;
  align-items: center;
  gap: 0.75rem;
  position: relative;
  border-radius: 0;
  backdrop-filter: blur(8px);
}

.sidebar-button:hover {
  background: var(--sidebar-btn-hover-bg);
  color: var(--color-text-primary);
  border-color: var(--color-border);
}

.sidebar-button-active {
  background: var(--sidebar-btn-active-bg);
  border-color: var(--color-border);
  color: var(--color-text-primary) !important;
  box-shadow: inset 0 1px 0 rgba(var(--rgb-contrast), 0.04);
}

.sidebar-button-active .sidebar-icon {
  color: var(--color-text-primary);
}

.active-indicator {
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  width: 6px;
  height: 1.75rem;
  background: linear-gradient(180deg, var(--indicator-gradient-start) 0%, var(--indicator-gradient-end) 100%);
  border-radius: 0 999px 999px 0;
  box-shadow: 0 0 16px var(--indicator-shadow);
}

.sidebar-divider {
  height: 1px;
  background: var(--color-border);
  margin: 1rem 0 0.85rem;
}

.sidebar-collapsed .sidebar-divider {
  margin: 0.75rem 0 0.6rem;
}

.sidebar-icon {
  min-width: 20px;
  flex-shrink: 0;
}

.sidebar-collapsed .sidebar-icon {
  margin: 0;
}

.sidebar-button-label {
  flex: 1;
  font-size: 0.92rem;
  font-weight: 600;
  line-height: 1;
  display: flex;
  align-items: center;
}

.sidebar-footer {
  position: relative;
  z-index: 1;
  padding: 0.7rem 0 0.85rem;
  border-top: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
  margin-top: auto;
}

.quit-button {
  justify-content: center;
  color: var(--color-danger);
  background: var(--danger-bg-weak);
  border-color: var(--danger-border);
}

.quit-button:hover {
  background: var(--danger-bg-hover);
  color: var(--danger-text-bright);
}

.sidebar-collapsed .quit-button {
  justify-content: center;
  padding: 0.8rem;
}

.sidebar-collapsed .sidebar-nav {
  padding-left: 0;
  padding-right: 0;
}

.sidebar-collapsed .sidebar-button {
  justify-content: center;
  padding: 0.25rem 0;
}

.sidebar-collapsed .active-indicator {
  left: 0;
}

@media (max-width: 720px) {
  .sidebar,
  .sidebar-collapsed {
    width: 100%;
    min-width: 100%;
    flex-basis: auto;
  }

  .sidebar {
    box-shadow: inset 0 -1px 0 var(--color-border);
  }

  .sidebar-nav {
    padding-bottom: 1.2rem;
  }
}
</style>
