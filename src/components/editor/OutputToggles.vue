<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from 'vue'
import { Mic, AudioLines, ChevronUp, ChevronDown, Check } from 'lucide-vue-next'
import { t } from '../../i18n'

/**
 * Editor action bar switches for the app-wide audio outputs (ROADMAP-142):
 * the virtual microphone and the VBAN network output. The popover opens
 * upward like the route menu; toggling a row keeps the menu open. Routing is
 * RouteSelector's job — these rows only enable or disable physical outputs.
 *
 * Menu semantics follow the shared `ui-menu` roles: real focus moves between
 * the row buttons (roving focus), Tab closes the menu, Escape returns focus
 * to the trigger.
 */
const props = defineProps<{
  virtualMicEnabled: boolean
  vbanEnabled: boolean
  /**
   * Whether the output currently has what it needs to feed: for the mic a
   * selected device that still exists in the live device list, for VBAN a
   * complete saved target. An unavailable output is disabled and greyed even
   * when the saved switch says enabled; the row explains the reason.
   */
  virtualMicAvailable: boolean
  vbanAvailable: boolean
  compact: boolean
  /** Settings snapshot has arrived; the switch never invents a state. */
  ready: boolean
  /** A persist-first toggle is in flight on one of the outputs. */
  pending: boolean
  /**
   * Popover direction: `up` for the editor action bar (bottom edge),
   * `down` for the main-window titlebar (top edge). Default keeps the
   * historical editor placement.
   */
  direction?: 'up' | 'down'
}>()

const emit = defineEmits<{
  /** The menu was opened: the owner refreshes availability data (device list). */
  open: []
  'toggle-virtual-mic': []
  'toggle-vban': []
}>()

type OutputRowId = 'virtual_mic' | 'vban'

interface OutputRow {
  id: OutputRowId
  label: string
  enabled: boolean
  available: boolean
}

const open = ref(false)
const triggerRef = ref<HTMLButtonElement | null>(null)
const rowRefs = ref<HTMLButtonElement[]>([])

const rows = computed<OutputRow[]>(() => [
  {
    id: 'virtual_mic',
    label: t('editor.outputs.mic_label'),
    enabled: props.virtualMicEnabled,
    available: props.virtualMicAvailable,
  },
  {
    id: 'vban',
    label: t('editor.outputs.vban_label'),
    enabled: props.vbanEnabled,
    available: props.vbanAvailable,
  },
])

const buttonAriaLabel = computed(() =>
  t('editor.outputs.button_aria', {
    mic: rows.value[0]!.enabled ? t('editor.outputs.on') : t('editor.outputs.off'),
    vban: rows.value[1]!.enabled ? t('editor.outputs.on') : t('editor.outputs.off'),
  }),
)

function unavailableHint(id: OutputRowId): string {
  return id === 'virtual_mic'
    ? t('editor.outputs.mic_unconfigured')
    : t('editor.outputs.vban_unconfigured')
}

function rowTitle(row: OutputRow): string {
  const state = t('editor.outputs.row_aria', {
    label: row.label,
    state: row.enabled ? t('editor.outputs.on') : t('editor.outputs.off'),
  })
  // The reason is shown even when the saved switch is off: greyed plus a
  // bare "off" would not explain that the output is unusable.
  return row.available ? state : `${state}. ${unavailableHint(row.id)}`
}

function setRowRef(el: unknown, index: number) {
  if (el instanceof HTMLButtonElement) rowRefs.value[index] = el
}

function focusRow(index: number) {
  rowRefs.value[index]?.focus()
}

async function openMenu() {
  if (!props.ready || open.value) return
  open.value = true
  // The owner refreshes availability data (device list) per menu open.
  emit('open')
  await nextTick()
  focusFirstAvailable()
}

function closeMenu(refocusTrigger: boolean) {
  if (!open.value) return
  open.value = false
  if (refocusTrigger) triggerRef.value?.focus()
}

function toggleMenu() {
  if (open.value) closeMenu(true)
  else void openMenu()
}

function toggleRow(row: OutputRow) {
  // The menu stays open: this is a switch panel, not a picker. The owner
  // (InputPanel) serialises requests and guards pending and unavailable
  // toggles — the same guards cover the editor hotkeys.
  if (!props.ready || !row.available) return
  if (row.id === 'virtual_mic') emit('toggle-virtual-mic')
  else emit('toggle-vban')
}

function onTriggerKeydown(event: KeyboardEvent) {
  if (open.value) {
    // Focus lives in the rows once the menu is open — unless every row is
    // disabled and the menu stays on the trigger for its explanation.
    if (event.key === 'Escape') {
      event.preventDefault()
      closeMenu(true)
    } else if (event.key === 'Tab') {
      // Let the browser move focus out naturally; close so no trap remains.
      closeMenu(false)
    } else if (event.key === 'ArrowDown') {
      // Async availability: rows may have become focusable after the menu
      // opened (the device list arrived) — arrows move focus in from the
      // trigger instead of leaving the menu keyboard-inaccessible.
      event.preventDefault()
      focusFirstAvailable()
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      focusLastAvailable()
    }
    return
  }
  if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault()
    void openMenu()
  }
}

/** Focus the nearest available row in `dir`, wrapping; unavailable rows are
 * skipped, and when none are available focus simply stays put. */
function focusNextAvailable(from: number, dir: 1 | -1) {
  const count = rows.value.length
  for (let step = 1; step <= count; step++) {
    const index = (((from + dir * step) % count) + count) % count
    if (rows.value[index]!.available) {
      focusRow(index)
      return
    }
  }
}

function focusFirstAvailable() {
  const index = rows.value.findIndex((row) => row.available)
  if (index !== -1) focusRow(index)
}

function focusLastAvailable() {
  for (let index = rows.value.length - 1; index >= 0; index--) {
    if (rows.value[index]!.available) {
      focusRow(index)
      return
    }
  }
}

function onRowKeydown(event: KeyboardEvent, index: number) {
  // Only available rows are focusable, so `index` is always an enabled one.
  switch (event.key) {
    case 'Escape':
      event.preventDefault()
      closeMenu(true)
      break
    case 'ArrowDown':
      event.preventDefault()
      focusNextAvailable(index, 1)
      break
    case 'ArrowUp':
      event.preventDefault()
      focusNextAvailable(index, -1)
      break
    case 'Home':
      event.preventDefault()
      focusFirstAvailable()
      break
    case 'End':
      event.preventDefault()
      focusLastAvailable()
      break
    case 'Tab':
      // Let the browser move focus out naturally; the menu must not linger.
      closeMenu(false)
      break
  }
}

function onDocumentClick(event: MouseEvent) {
  if (!open.value) return
  const target = event.target as HTMLElement | null
  if (target && !target.closest('.output-toggles')) {
    closeMenu(false)
  }
}

onMounted(() => document.addEventListener('click', onDocumentClick))
onUnmounted(() => document.removeEventListener('click', onDocumentClick))
</script>

<template>
  <div class="output-toggles" :class="{ compact }">
    <button
      ref="triggerRef"
      type="button"
      class="output-toggles-btn"
      :class="{ 'is-open': open }"
      :disabled="!ready"
      aria-haspopup="menu"
      :aria-expanded="open"
      :aria-busy="pending"
      :title="buttonAriaLabel"
      :aria-label="buttonAriaLabel"
      @click.stop="toggleMenu"
      @keydown="onTriggerKeydown"
    >
      <span class="out-icon-wrap" :class="{ 'is-off': !virtualMicEnabled || !virtualMicAvailable }">
        <Mic :size="14" />
      </span>
      <span class="out-icon-wrap" :class="{ 'is-off': !vbanEnabled || !vbanAvailable }">
        <AudioLines :size="14" />
      </span>
      <ChevronUp
        v-if="direction !== 'down'"
        :size="14"
        class="chevron"
        :class="{ 'chevron-open': open }"
      />
      <ChevronDown
        v-else
        :size="14"
        class="chevron"
        :class="{ 'chevron-open': open }"
      />
    </button>

    <ul
      v-if="open"
      class="output-menu ui-menu"
      :class="direction === 'down' ? 'output-menu--down' : 'output-menu--up'"
      role="menu"
      :aria-label="t('editor.outputs.menu_aria')"
    >
      <li v-for="(row, index) in rows" :key="row.id" role="none">
        <button
          type="button"
          :ref="(el) => setRowRef(el, index)"
          class="output-row ui-menu-item"
          role="menuitemcheckbox"
          :aria-checked="row.enabled"
          :aria-label="rowTitle(row)"
          :title="rowTitle(row)"
          tabindex="-1"
          :disabled="!ready || !row.available"
          @click="toggleRow(row)"
          @keydown="onRowKeydown($event, index)"
        >
          <span class="row-icon-wrap" :class="{ 'is-off': !row.enabled }">
            <Mic v-if="row.id === 'virtual_mic'" :size="14" />
            <AudioLines v-else :size="14" />
          </span>
          <span class="row-body">
            <span class="row-label">{{ row.label }}</span>
          </span>
          <span
          class="row-check"
          :class="{ 'is-checked': row.enabled && row.available, 'is-unavailable': !row.available }"
          aria-hidden="true"
        >
            <Check v-if="row.enabled" :size="12" />
          </span>
        </button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.output-toggles {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

/* Neutral hover like the typing toggle: this group indicates output state,
   it is not a transient action (overrides the shared accent fill habit of
   the action bar while keeping the open state visible). */
.output-toggles-btn {
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

.output-toggles-btn:hover:not(:disabled),
.output-toggles-btn.is-open {
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  border-color: var(--color-accent);
}

.output-toggles-btn:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.output-toggles.compact .output-toggles-btn {
  padding: 0.3rem 0.55rem;
  gap: 0.2rem;
}

/* A switched-off output shows a muted icon with the shared diagonal strike
   (same language as the typing toggle). The strike sits on the wrapping span:
   ::after does not render on SVG elements. */
.out-icon-wrap,
.row-icon-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.out-icon-wrap.is-off,
.row-icon-wrap.is-off {
  opacity: 0.45;
}

.out-icon-wrap.is-off::after,
.row-icon-wrap.is-off::after {
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

.chevron {
  flex-shrink: 0;
  opacity: 0.8;
  transition: transform 0.15s ease;
}

.chevron-open {
  transform: rotate(180deg);
}

/* Panel form (padding/border/background) comes from the shared ui-menu role;
   this block keeps only the popover placement: up for the editor action bar
   (bottom edge), down for the main-window titlebar (top edge). Both anchor to
   the trigger's outer edge so the menu stays inside the window. */
.output-menu {
  position: absolute;
  right: 0;
  z-index: 1000;
  width: max-content;
  max-width: min(18rem, calc(100vw - 2rem));
}

.output-menu--up {
  bottom: calc(100% + 4px);
}

.output-menu--down {
  top: calc(100% + 4px);
}

/* Row layout over the shared ui-menu-item role: the shared class owns the
   neutral hover, focus outline and disabled look; only the flex layout of the
   icon + label + checkbox is local. */
.output-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  white-space: nowrap;
  cursor: pointer;
}

.row-body {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
  min-width: 0;
  text-align: left;
}

.row-label {
  font-size: 0.8rem;
}

/* Visual checkbox mirroring the checked state carried by aria-checked. */
.row-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  margin-left: auto;
  border: 1px solid var(--color-border-strong);
  border-radius: 4px;
  background: var(--color-bg-field);
  color: transparent;
}

.row-check.is-checked {
  background: var(--color-accent);
  border-color: var(--color-accent);
  color: var(--color-text-on-accent, #ffffff);
}

/* Unavailable row: the checkbox stays muted — no blue fill, no visible
   check, on top of the shared ui-menu-item disabled look. */
.row-check.is-unavailable {
  opacity: 0.4;
  background: var(--color-bg-field);
  border-color: var(--color-border);
  color: transparent;
}

@media (prefers-reduced-motion: reduce) {
  .chevron {
    transition: none;
  }
}
</style>
