<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import { useInterfaceFontSync } from '../src/composables/useInterfaceFontSync';
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { open, confirm } from '@tauri-apps/plugin-dialog'
import { t } from '../src/i18n'
import { createAsyncCleanupScope } from '../src/utils/asyncCleanup'
import { registerSoundPanelAppListeners, installSoundPanelKeydown } from '../src/playback/listeners'

useInterfaceFontSync();

interface SoundBinding {
  key: string
  description: string
  filename: string
}

interface SoundSet {
  id: string
  name: string
  bindings: SoundBinding[]
}

interface SoundSets {
  active_set_id: string
  sets: SoundSet[]
}

const noBindingMessage = ref<string | null>(null)
let messageTimeout: number | null = null

const opacity = ref(90)
const bgColor = ref('#2a2a2a')
const clickthroughEnabled = ref(false)
const stayVisible = ref(false)

interface Binding {
  key: string
  description: string
  filename: string
}

const bindings = ref<Binding[]>([])

const activeSetId = ref<string>('')
const activeSetName = ref<string>('')
const sets = ref<SoundSet[]>([])
const showSetMenu = ref(false)
const setDropdownRef = ref<HTMLElement | null>(null)

const mode = ref<'runtime' | 'config'>('runtime')

const KEYBOARD_ROWS: string[][] = [
  ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
  ['Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P'],
  ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L'],
  ['Z', 'X', 'C', 'V', 'B', 'N', 'M'],
]

const focusedKey = ref('')
const keyRefs = new Map<string, HTMLElement>()

const showConfigDialog = ref(false)
const configKey = ref('')
const configDescription = ref('')
const configFilePath = ref('')

const showAddSetDialog = ref(false)
const newSetName = ref('')
const addSetInput = ref<HTMLInputElement | null>(null)
const isAddingSet = ref(false)

watch(showAddSetDialog, (visible) => {
  if (visible) {
    setTimeout(() => addSetInput.value?.focus(), 0)
  }
})

function toggleMode() {
  mode.value = mode.value === 'runtime' ? 'config' : 'runtime'
}

async function toggleStayVisible() {
  const previous = stayVisible.value
  stayVisible.value = !stayVisible.value
  try {
    await invoke('sp_set_stay_visible', { enabled: stayVisible.value })
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    stayVisible.value = previous
    console.error('[SoundPanel] Failed to toggle stay_visible:', e)
  }
}

watch(mode, (newMode) => {
  invoke('sp_set_config_mode', { enabled: newMode === 'config' }).catch(e => {
    console.error('[SoundPanel] Failed to set config_mode:', e)
  })
}, { immediate: true })

const overlayStyle = computed(() => {
  const base = hexToRgba(bgColor.value, opacity.value / 100)
  return {
    backgroundColor: base,
    '--panel-bg': bgColor.value,
  }
})

const isLightBackground = computed(() => {
  const r = parseInt(bgColor.value.slice(1, 3), 16)
  const g = parseInt(bgColor.value.slice(3, 5), 16)
  const b = parseInt(bgColor.value.slice(5, 7), 16)
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.55
})

function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16)
  const g = parseInt(hex.slice(3, 5), 16)
  const b = parseInt(hex.slice(5, 7), 16)
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

async function loadBindings() {
  try {
    const loadedBindings = await invoke<SoundBinding[]>('sp_get_bindings')
    bindings.value = loadedBindings.map(b => ({
      key: b.key,
      description: b.description,
      filename: b.filename
    }))
    console.log('[SoundPanel] Loaded bindings:', bindings.value)
  } catch (e) {
    console.error('[SoundPanel] Failed to load bindings:', e)
  }
}

async function loadSets() {
  try {
    const result = await invoke<SoundSets>('sp_get_sets')
    sets.value = result.sets || []
    activeSetId.value = result.active_set_id || ''
    const active = sets.value.find(s => s.id === activeSetId.value)
    activeSetName.value = active ? active.name : ''
  } catch (e) {
    console.error('[SoundPanel] Failed to load sets:', e)
  }
}

async function loadStayVisible() {
  try {
    stayVisible.value = await invoke<boolean>('sp_get_stay_visible')
    console.log('[SoundPanel] Loaded stay_visible:', stayVisible.value)
  } catch (e) {
    console.error('Failed to load stay_visible:', e)
  }
}

function getNextSetIdx(): number {
  const idx = sets.value.findIndex(s => s.id === activeSetId.value)
  if (idx < 0) return 0
  return (idx + 1) % sets.value.length
}

function getPrevSetIdx(): number {
  const idx = sets.value.findIndex(s => s.id === activeSetId.value)
  if (idx < 0) return 0
  return (idx - 1 + sets.value.length) % sets.value.length
}

async function cycleSet(direction: 'next' | 'prev') {
  if (sets.value.length <= 1) return
  showSetMenu.value = false
  const idx = direction === 'next' ? getNextSetIdx() : getPrevSetIdx()
  const newId = sets.value[idx].id
  await invoke('sp_set_active_set', { id: newId })
}

async function onSelectSet(id: string) {
  if (id === activeSetId.value) return
  await invoke('sp_set_active_set', { id })
  ;(document.activeElement as HTMLElement | null)?.blur()
}

async function selectSetFromMenu(id: string) {
  showSetMenu.value = false
  await onSelectSet(id)
}

function onSetDropdownFocusOut(event: FocusEvent) {
  const nextTarget = event.relatedTarget as Node | null
  if (!nextTarget || !setDropdownRef.value?.contains(nextTarget)) {
    showSetMenu.value = false
  }
}

async function onAddSet() {
  newSetName.value = ''
  showAddSetDialog.value = true
}

async function confirmAddSet() {
  const name = newSetName.value.trim()
  if (!name || isAddingSet.value) return
  isAddingSet.value = true
  try {
    const created = await invoke<SoundSet>('sp_add_set', { name })
    await loadSets()
    activeSetId.value = created.id
    showAddSetDialog.value = false
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    console.error('[SoundPanel] Failed to add set:', e)
  } finally {
    isAddingSet.value = false
  }
}

async function onRemoveSet() {
  if (sets.value.length <= 1) return
  const set = sets.value.find(s => s.id === activeSetId.value)
  const name = set ? set.name : ''
  const confirmedResult = await confirm(t('soundpanel.remove_set.message', { name }), {
    title: t('soundpanel.remove_set.title'),
    kind: 'warning'
  })
  if (!confirmedResult) return
  try {
    await invoke('sp_remove_set', { id: activeSetId.value })
    await loadSets()
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    console.error('[SoundPanel] Failed to remove set:', e)
  }
}

function showNoBinding(key: string) {
  noBindingMessage.value = t('soundpanel.no_binding', { key })

  if (messageTimeout !== null) {
    clearTimeout(messageTimeout)
  }

  messageTimeout = window.setTimeout(() => {
    noBindingMessage.value = null
    messageTimeout = null
  }, 2000)
}

async function closeWindow() {
  try {
    await invoke('close_soundpanel_window')
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    console.error('Failed to close window:', e)
  }
}

async function escapeWindow() {
  try {
    await invoke('sp_escape_soundpanel')
  } catch (e) {
    console.error('Failed to leave sound panel:', e)
  }
}

function isBound(key: string): boolean {
  return bindings.value.some(b => b.key === key)
}

function bindingDesc(key: string): string {
  const b = bindings.value.find(x => x.key === key)
  return b ? b.description : ''
}

function bindingTitle(key: string): string {
  const b = bindings.value.find(x => x.key === key)
  return b
    ? t('soundpanel.binding.assigned', { key, description: b.description })
    : t('soundpanel.binding.free', { key })
}

function onKeyActivate(key: string) {
  if (mode.value === 'config') {
    openConfigDialog(key)
  } else {
    if (isBound(key)) {
      playBinding(key)
    } else {
      showNoBinding(key)
    }
  }
}

function setKeyRef(key: string, el: HTMLElement | null) {
  if (el) {
    keyRefs.set(key, el)
  } else {
    keyRefs.delete(key)
  }
}

async function toggleClickthrough() {
  const previous = clickthroughEnabled.value
  const next = !previous
  clickthroughEnabled.value = next
  try {
    await invoke('sp_set_floating_clickthrough', { enabled: next })
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    clickthroughEnabled.value = previous
    console.error('[SoundPanel] Failed to toggle clickthrough:', e)
  }
}

function codeToKey(code: string): string | null {
  if (code.length === 4 && code.startsWith('Key')) {
    const letter = code[3].toUpperCase()
    if (letter >= 'A' && letter <= 'Z') return letter
  }
  if (code.startsWith('Digit') && code.length === 6) {
    const digit = code[5]
    if (digit >= '0' && digit <= '9') return digit
  }
  return null
}

function playBinding(key: string) {
  invoke('sp_play_binding', { key }).catch(e => {
    console.error('[SoundPanel] Failed to play binding:', e)
  })
  ;(document.activeElement as HTMLElement | null)?.blur()
}

function openConfigDialog(key: string) {
  const existing = bindings.value.find(x => x.key === key)
  configKey.value = key
  configDescription.value = existing ? existing.description : ''
  configFilePath.value = existing ? existing.filename : ''
  showConfigDialog.value = true
}

async function pickFile() {
  try {
    const result = await open({
      title: t('soundpanel.file_picker.title'),
      multiple: false,
      filters: [
        {
          name: t('soundpanel.file_picker.filter'),
          extensions: ['mp3', 'wav', 'ogg', 'flac']
        }
      ]
    })
    if (result) {
      configFilePath.value = typeof result === 'string' ? result : String(result)
    }
  } catch (e) {
    console.error('[SoundPanel] Failed to pick file:', e)
  }
}

async function saveConfigBinding() {
  if (!configKey.value || !configDescription.value || !configFilePath.value) return
  try {
    const existing = bindings.value.find(x => x.key === configKey.value)
    if (existing) {
      const fileChanged = configFilePath.value !== existing.filename
      await invoke('sp_update_binding', {
        key: configKey.value,
        description: configDescription.value.trim(),
        filePath: fileChanged ? configFilePath.value : null
      })
    } else {
      await invoke('sp_add_binding', {
        key: configKey.value,
        description: configDescription.value.trim(),
        filePath: configFilePath.value
      })
    }
    showConfigDialog.value = false
    ;(document.activeElement as HTMLElement | null)?.blur()
  } catch (e) {
    console.error('[SoundPanel] Failed to save binding:', e)
  }
}

function moveFocus(direction: string) {
  let currentKey = focusedKey.value
  if (!currentKey) {
    currentKey = KEYBOARD_ROWS[0][0]
  }

  let row = -1
  let col = -1
  for (let r = 0; r < KEYBOARD_ROWS.length; r++) {
    const c = KEYBOARD_ROWS[r].indexOf(currentKey)
    if (c !== -1) {
      row = r
      col = c
      break
    }
  }
  if (row === -1) {
    focusedKey.value = KEYBOARD_ROWS[0][0]
    keyRefs.get(KEYBOARD_ROWS[0][0])?.focus()
    return
  }

  const numRows = KEYBOARD_ROWS.length

  switch (direction) {
    case 'ArrowRight': {
      const currentRow = KEYBOARD_ROWS[row]
      if (col + 1 < currentRow.length) {
        col = col + 1
      } else {
        row = (row + 1) % numRows
        col = 0
      }
      break
    }
    case 'ArrowLeft': {
      if (col > 0) {
        col = col - 1
      } else {
        row = (row - 1 + numRows) % numRows
        col = KEYBOARD_ROWS[row].length - 1
      }
      break
    }
    case 'ArrowDown': {
      row = (row + 1) % numRows
      col = Math.min(col, KEYBOARD_ROWS[row].length - 1)
      break
    }
    case 'ArrowUp': {
      row = (row - 1 + numRows) % numRows
      col = Math.min(col, KEYBOARD_ROWS[row].length - 1)
      break
    }
    default:
      return
  }

  const newKey = KEYBOARD_ROWS[row][col]
  focusedKey.value = newKey
  keyRefs.get(newKey)?.focus()
}

async function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (showSetMenu.value) {
      showSetMenu.value = false
      return
    }
    if (showConfigDialog.value) {
      showConfigDialog.value = false
      return
    }
    if (showAddSetDialog.value) {
      showAddSetDialog.value = false
      return
    }
    escapeWindow()
    return
  }
  if (showConfigDialog.value || showAddSetDialog.value) {
    return
  }
  if (!e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
    const functionKey = /^F([1-9]|1[0-2])$/.exec(e.key)
    if (functionKey) {
      e.preventDefault()
      const targetSet = sets.value[Number(functionKey[1]) - 1]
      if (targetSet && targetSet.id !== activeSetId.value) {
        await onSelectSet(targetSet.id)
      }
      return
    }
  }
  if (e.ctrlKey === true && !e.shiftKey && !e.altKey && !e.metaKey && e.code === 'KeyB') {
    e.preventDefault()
    toggleMode()
    return
  }
  if (e.key === 'PageUp' || e.key === 'PageDown') {
    if (e.ctrlKey || e.shiftKey || e.altKey || e.metaKey) {
      return
    }
    e.preventDefault()
    await cycleSet(e.key === 'PageDown' ? 'next' : 'prev')
    ;(document.activeElement as HTMLElement | null)?.blur()
    return
  }
  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown') {
    if (e.ctrlKey || e.shiftKey || e.altKey || e.metaKey) {
      return
    }
    e.preventDefault()
    moveFocus(e.key)
    return
  }
  if (e.ctrlKey || e.shiftKey || e.altKey || e.metaKey) {
    return
  }
  const key = codeToKey(e.code)
  if (!key) return
  if (mode.value === 'config') {
    e.preventDefault()
    openConfigDialog(key)
    return
  }
  const b = bindings.value.find(x => x.key === key)
  if (b) {
    e.preventDefault()
    playBinding(key)
  } else {
    showNoBinding(key)
  }
}

defineExpose({
  showNoBinding
})

const listenerScope = createAsyncCleanupScope()

onUnmounted(() => {
  listenerScope.dispose()
  window.removeEventListener('keydown', onKeydown)
  if (messageTimeout !== null) {
    clearTimeout(messageTimeout)
    messageTimeout = null
  }
})

onMounted(async () => {
  await loadSets()
  await loadBindings()
  await loadStayVisible()

  try {
    const [loadedOpacity, loadedColor] = await invoke<[number, string]>('sp_get_floating_appearance')
    console.log('[SoundPanel] Loaded appearance:', { opacity: loadedOpacity, color: loadedColor })
    opacity.value = loadedOpacity
    bgColor.value = loadedColor
  } catch (e) {
    console.error('Failed to load appearance:', e)
  }

  try {
    clickthroughEnabled.value = await invoke<boolean>('sp_is_floating_clickthrough_enabled')
  } catch (e) {
    console.error('Failed to load clickthrough:', e)
  }

  async function onAppearanceUpdate() {
    console.log('[SoundPanel] Appearance update event received')
    const [newOpacity, newColor] = await invoke<[number, string]>('sp_get_floating_appearance')
    console.log('[SoundPanel] New appearance:', { opacity: newOpacity, color: newColor })
    opacity.value = newOpacity
    bgColor.value = newColor
    try {
      clickthroughEnabled.value = await invoke<boolean>('sp_is_floating_clickthrough_enabled')
    } catch (e) {
      console.error('Failed to reload clickthrough:', e)
    }
    await loadStayVisible()
    console.log('[SoundPanel] Updated refs:', { opacity: opacity.value, bgColor: bgColor.value })
  }

  async function onBindingsOrSetChanged() {
    console.log('[SoundPanel] Bindings or active set changed event received, reloading')
    await loadSets()
    await loadBindings()
  }

  try {
    await registerSoundPanelAppListeners(listen, listenerScope, {
      onAppearanceUpdate,
      onBindingsChanged: onBindingsOrSetChanged,
      onActiveSetChanged: onBindingsOrSetChanged,
    })
    await listenerScope.track(
      listen<string>('no-binding', (event) => {
        showNoBinding(event.payload)
      }),
    )
    installSoundPanelKeydown(listenerScope, onKeydown)
  } catch (e) {
    console.error('Failed to register soundpanel listeners:', e)
  }

  ;(document.activeElement as HTMLElement | null)?.blur()
})
</script>

<template>
  <div class="overlay" :class="{ 'light-background': isLightBackground }" :style="overlayStyle">
    <div class="title-bar" data-tauri-drag-region>
      <div class="title-left">
        <span class="title">SoundPanel</span>
        <span
          v-if="stayVisible"
          class="persistent-mode-label"
          :title="t('soundpanel.pin.tooltip')"
          :aria-label="t('soundpanel.pin.aria')"
        >{{ t('soundpanel.pin.label') }}</span>
      </div>
      <div v-if="sets.length > 0" class="set-selector">
        <button
          v-if="sets.length > 1"
          class="set-arrow"
          @click="cycleSet('prev')"
          :title="t('soundpanel.set.prev')"
        >&#9664;</button>
        <div
          ref="setDropdownRef"
          class="set-dropdown"
          @focusout="onSetDropdownFocusOut"
        >
          <button
            type="button"
            class="set-select"
            role="combobox"
            aria-haspopup="listbox"
            :aria-expanded="showSetMenu"
            :title="t('soundpanel.set.select_title', { name: activeSetName })"
            @click="showSetMenu = !showSetMenu"
          >
            <span class="set-select-label">{{ activeSetName }}</span>
          </button>
          <div v-if="showSetMenu" class="set-menu" role="listbox" :aria-label="t('soundpanel.set.menu_aria')">
            <button
              v-for="s in sets"
              :key="s.id"
              type="button"
              class="set-option"
              :class="{ selected: s.id === activeSetId }"
              role="option"
              :aria-selected="s.id === activeSetId"
              :title="s.name"
              @click="selectSetFromMenu(s.id)"
            >{{ s.name }}</button>
          </div>
        </div>
        <button
          v-if="sets.length > 1"
          class="set-arrow"
          @click="cycleSet('next')"
          :title="t('soundpanel.set.next')"
        >&#9654;</button>
        <template v-if="mode === 'config'">
          <button
            class="set-arrow"
            @click="onAddSet"
            :title="t('soundpanel.set.add')"
          >+</button>
          <button
            v-if="sets.length > 1"
            class="set-arrow"
            @click="onRemoveSet"
            :title="t('soundpanel.set.remove')"
          >&#215;</button>
        </template>
      </div>
      <div class="buttons">
        <button
          class="mode-toggle"
          :class="{ 'mode-config': mode === 'config' }"
          @click="toggleMode"
          :title="mode === 'runtime' ? t('soundpanel.mode.runtime_title') : t('soundpanel.mode.config_title')"
          :aria-label="mode === 'runtime' ? t('soundpanel.mode.runtime_title') : t('soundpanel.mode.config_title')"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.28z"/>
            <path d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
          </svg>
        </button>
        <button
          class="mode-toggle pin-toggle"
          :class="{ 'panel-pinned': stayVisible }"
          @click="toggleStayVisible"
          :title="stayVisible ? t('soundpanel.pin_toggle.pinned') : t('soundpanel.pin_toggle.unpinned')"
          :aria-label="stayVisible ? t('soundpanel.pin_toggle.pinned') : t('soundpanel.pin_toggle.unpinned')"
        >
          <svg v-if="stayVisible" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 17v5M5 17h14M15 4.5V2H9v2.5a2 2 0 01-.4 1.2L7 8h10l-1.6-2.3a2 2 0 01-.4-1.2zM6 8l-1 9h14l-1-9"/>
          </svg>
          <svg v-else width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 17v5M5 17h14M15 4.5V2H9v2.5a2 2 0 01-.4 1.2L7 8h10l-1.6-2.3a2 2 0 01-.4-1.2zM6 8l-1 9h14l-1-9"/>
            <line x1="2" y1="2" x2="22" y2="22"/>
          </svg>
        </button>
        <button class="close-btn" @click="closeWindow" :title="t('soundpanel.close.title')" :aria-label="t('common.close')">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </div>

    <div class="content" :class="{ 'clickthrough': clickthroughEnabled }">
      <div v-if="noBindingMessage" class="no-binding-message">
        {{ noBindingMessage }}
      </div>

      <div v-else class="content-inner">
        <div v-if="bindings.length > 0 || mode === 'config'" class="bindings-list">
          <div class="keyboard">
            <div v-for="row in KEYBOARD_ROWS" :key="row[0]" class="kb-row">
              <button
                v-for="key in row"
                :key="key"
                type="button"
                class="kb-key"
                :class="{ bound: isBound(key), focused: focusedKey === key }"
                :title="bindingTitle(key)"
                :ref="(el: any) => setKeyRef(key, el as HTMLElement | null)"
                @click="onKeyActivate(key)"
              >
                <span class="kb-cap">{{ key }}</span>
                <span v-if="isBound(key)" class="kb-desc">{{ bindingDesc(key) }}</span>
              </button>
            </div>
          </div>
        </div>

        <div v-else class="hint-message">
          <div>{{ t('soundpanel.empty.title') }}</div>
          <div class="hint-sub">
            {{ t('soundpanel.empty.hint') }}
          </div>
        </div>

      </div>
    </div>

    <div v-if="showAddSetDialog" class="config-dialog-overlay">
      <div class="config-dialog">
        <div class="config-dialog-title">{{ t('soundpanel.add_set.title') }}</div>
        <input
          ref="addSetInput"
          v-model="newSetName"
          type="text"
          class="config-input"
          :placeholder="t('soundpanel.add_set.name_placeholder')"
          @keydown.enter="confirmAddSet"
        />
        <div class="config-actions">
          <button
            class="config-save"
            :disabled="!newSetName.trim() || isAddingSet"
            @click="confirmAddSet"
          >{{ isAddingSet ? t('soundpanel.add_set.adding') : t('soundpanel.add_set.add') }}</button>
          <button class="config-cancel" @click="showAddSetDialog = false">{{ t('common.cancel') }}</button>
        </div>
      </div>
    </div>

    <div v-if="showConfigDialog" class="config-dialog-overlay">
      <div class="config-dialog">
        <div class="config-dialog-title">{{ t('soundpanel.config.title', { key: configKey }) }}</div>
        <input
          v-model="configDescription"
          type="text"
          class="config-input"
          :placeholder="t('soundpanel.config.description_placeholder')"
        />
        <div class="config-file-row">
          <span class="config-file-path">{{ configFilePath || t('soundpanel.config.no_file') }}</span>
          <button class="config-browse" @click="pickFile">{{ t('soundpanel.config.browse') }}</button>
        </div>
        <div class="config-actions">
          <button
            class="config-save"
            :disabled="!configDescription || !configFilePath"
            @click="saveConfigBinding"
          >{{ t('common.save') }}</button>
          <button class="config-cancel" @click="showConfigDialog = false">{{ t('common.cancel') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html {
  --font-sans: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  font-size: var(--ui-font-size, 16px);
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

body {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  font-family: var(--ui-font-family, var(--font-sans));
  background: transparent;
  overflow: hidden;
}

#app {
  width: 100%;
  height: 100%;
  overflow: hidden;
}

::-webkit-scrollbar {
  display: none;
}
</style>

<style scoped>
.overlay {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  overflow: hidden;
  --panel-text: #ffffff;
  --panel-muted: rgba(255, 255, 255, 0.7);
  --panel-border: rgba(255, 255, 255, 0.2);
}

.overlay.light-background {
  --panel-text: #1f2937;
  --panel-muted: rgba(31, 41, 55, 0.7);
  --panel-border: rgba(31, 41, 55, 0.2);
}

.title-bar {
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px;
  user-select: none;
  -webkit-app-region: drag;
  position: relative;
}

.title-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.title {
  font-size: 0.8rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  color: var(--panel-muted);
}

.persistent-mode-label {
  flex-shrink: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 1px 6px;
  font-size: 9px;
  line-height: 1.4;
  font-weight: 400;
  color: var(--panel-muted);
  background: color-mix(in srgb, var(--panel-text) 6%, transparent);
  border: 1px solid var(--panel-border);
  border-radius: 3px;
  -webkit-app-region: no-drag;
  user-select: none;
}

.set-selector {
  display: flex;
  align-items: center;
  gap: 4px;
  -webkit-app-region: no-drag;
  position: absolute;
  left: 50%;
  transform: translateX(-50%);
}

.set-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--panel-text);
  opacity: 0.85;
  min-width: 60px;
  text-align: center;
}

.set-arrow {
  background: transparent;
  border: 1px solid var(--panel-border);
  color: var(--panel-muted);
  width: 20px;
  height: 20px;
  border-radius: 3px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  padding: 0;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}

.set-arrow:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: var(--panel-text);
  color: var(--panel-text);
}

.mode-toggle {
  background: transparent;
  border: 1px solid var(--panel-border);
  color: var(--panel-muted);
  width: 24px;
  height: 24px;
  border-radius: 3px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  padding: 0;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
  -webkit-app-region: no-drag;
}

.mode-toggle:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: var(--panel-text);
  color: var(--panel-text);
}

.mode-toggle.mode-config {
  background: rgba(100, 160, 255, 0.3);
}

.mode-toggle.panel-pinned {
  background: rgba(100, 160, 255, 0.3);
}

.set-select {
  background-color: transparent;
  background-image:
    linear-gradient(45deg, transparent 50%, currentColor 50%),
    linear-gradient(135deg, currentColor 50%, transparent 50%);
  background-position:
    calc(100% - 8px) calc(50% - 1px),
    calc(100% - 5px) calc(50% - 1px);
  background-repeat: no-repeat;
  background-size: 3px 3px;
  color: var(--panel-text);
  border: 1px solid var(--panel-border);
  border-radius: 3px;
  font-size: 12px;
  font-weight: 500;
  padding: 2px 16px 2px 6px;
  width: 140px;
  height: 22px;
  justify-content: flex-start;
  text-align: left;
  cursor: pointer;
  -webkit-app-region: no-drag;
}

.set-select-label {
  display: block;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.set-select:hover { border-color: var(--panel-text); }

.set-select:focus {
  outline: none;
}

.set-dropdown {
  position: relative;
  width: 140px;
  -webkit-app-region: no-drag;
}

.set-menu {
  position: absolute;
  top: calc(100% + 3px);
  left: 0;
  z-index: 50;
  width: 100%;
  max-height: 156px;
  overflow-y: auto;
  padding: 3px;
  background: color-mix(in srgb, var(--panel-bg) 24%, transparent);
  border: 1px solid var(--panel-border);
  border-radius: 4px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
  backdrop-filter: blur(8px);
}

.set-option {
  width: 100%;
  height: 24px;
  padding: 2px 6px;
  justify-content: flex-start;
  overflow: hidden;
  border-color: transparent;
  border-radius: 3px;
  color: var(--panel-text);
  font-size: 12px;
  font-weight: 500;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.set-option:hover,
.set-option:focus-visible {
  background: color-mix(in srgb, var(--panel-text) 12%, transparent);
  border-color: var(--panel-border);
  outline: none;
}

.set-option.selected {
  background: rgba(100, 160, 255, 0.28);
}

.buttons {
  display: flex;
  gap: 6px;
  -webkit-app-region: no-drag;
}

button {
  background: transparent;
  border: 1px solid var(--panel-border);
  color: var(--panel-text);
  width: 24px;
  height: 24px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s, border-color 0.15s;
  padding: 0;
}

button:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: var(--panel-text);
}

.close-btn {
  width: 30px;
  height: 26px;
  border: none;
  border-radius: 6px;
  color: var(--panel-muted);
  transition: background 0.15s ease, color 0.15s ease;
}

.close-btn:hover {
  background: #e5484d;
  border-color: transparent;
  color: #fff;
}

button:active {
  background: rgba(255, 255, 255, 0.15);
}

button.active {
  background: rgba(100, 200, 100, 0.5);
  border-color: rgba(100, 200, 100, 0.8);
}

.content {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  -webkit-app-region: no-drag;
}

.content.clickthrough {
  pointer-events: none;
}

.content-inner {
  text-align: center;
}

.no-binding-message {
  color: #ff6b6b;
  font-size: 1.2rem;
  text-align: center;
  animation: shake 0.3s;
  padding: 1rem;
}

.bindings-list {
  padding: 1rem;
}

.bindings-title {
  font-size: 1rem;
  margin-bottom: 1rem;
  color: #aaa;
}

.keyboard {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.kb-row {
  display: flex;
  gap: 4px;
  justify-content: center;
}

.kb-key {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  gap: 2px;
  width: 56px;
  height: 56px;
  padding: 4px 2px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid var(--panel-border);
  border-radius: 6px;
  transition: background 0.15s, border-color 0.15s;
  cursor: pointer;
  font-family: inherit;
  font-size: inherit;
  color: inherit;
  outline: none;
}

.kb-key:hover {
  background: rgba(255, 255, 255, 0.1);
}

.kb-key:focus-visible,
.kb-key.focused {
  border-color: var(--panel-text);
  outline: 2px solid color-mix(in srgb, var(--panel-text) 40%, transparent);
  outline-offset: 1px;
}

.kb-key.bound {
  background: rgba(100, 200, 100, 0.15);
  border-color: rgba(100, 200, 100, 0.4);
}

.kb-key.bound:hover {
  background: rgba(100, 200, 100, 0.25);
}

.kb-cap {
  font-family: monospace;
  font-weight: bold;
  font-size: 1.1rem;
  color: var(--panel-text);
}

.kb-desc {
  font-size: 0.65rem;
  color: var(--panel-muted);
  max-width: 52px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hint-message {
  text-align: center;
  line-height: 1.6;
  font-size: 1rem;
  color: var(--panel-muted);
}

.hint-sub {
  margin-top: 0.5rem;
  font-size: 0.85rem;
  color: var(--panel-muted);
}

@keyframes shake {
  0%, 100% { transform: translateX(0); }
  25% { transform: translateX(-5px); }
  75% { transform: translateX(5px); }
}

.config-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
  -webkit-app-region: no-drag;
}

.config-dialog {
  --dlg-text: var(--panel-text);
  --dlg-muted: var(--panel-muted);
  --dlg-border: var(--panel-border);
  background: var(--panel-bg);
  border: 1px solid var(--dlg-border);
  border-radius: 8px;
  padding: 1rem;
  width: 320px;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  color: var(--dlg-text);
}

.config-dialog-title {
  font-size: 13px;
  font-weight: 500;
}

.config-input {
  appearance: none;
  -webkit-appearance: none;
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid var(--dlg-border);
  border-radius: 4px;
  color: var(--dlg-text);
  padding: 0.4rem 0.5rem;
  font-size: 12px;
  font-family: inherit;
  outline: none;
}

.config-input:focus {
  border-color: var(--dlg-text);
}

.config-file-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.config-file-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: var(--dlg-muted);
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid var(--dlg-border);
  border-radius: 4px;
  padding: 0.4rem 0.5rem;
}

.config-browse {
  flex-shrink: 0;
  width: auto;
  padding: 0.35rem 0.7rem;
  font-size: 12px;
}

.config-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}

.config-actions button {
  width: auto;
  padding: 0.35rem 0.9rem;
  font-size: 12px;
}

.config-actions button:disabled {
  opacity: 0.5;
  cursor: default;
}

.config-save {
  background: rgba(100, 200, 100, 0.3);
}

</style>
