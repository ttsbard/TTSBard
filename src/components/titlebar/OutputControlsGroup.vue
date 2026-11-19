<script setup lang="ts">
import { useAppSettings } from '../../composables/useAppSettings'
import { getMainWindowOutputControls } from '../../composables/useMainWindowOutputControls'
import OutputToggles from '../editor/OutputToggles.vue'

/**
 * Presentational host for the shared main-window output controls. All state
 * and the root keydown listener live in the root-lifetime controller
 * (`useMainWindowOutputControls`); this component only renders the popover
 * group wherever the placement preference puts it — the titlebar (down) or
 * the editor action bar (up). Mounting/moving/unmounting this component
 * never duplicates state or listeners.
 */
const props = withDefaults(
  defineProps<{
    /** Popover direction: down from the titlebar, up from the editor bar. */
    direction?: 'up' | 'down'
    compact?: boolean
    /** Tiny mono trigger; the menu stays full size. */
    micro?: boolean
  }>(),
  { direction: 'up', compact: false, micro: false },
)

const controls = getMainWindowOutputControls(useAppSettings())
</script>

<template>
  <OutputToggles
    class="output-controls-group"
    :class="props.direction === 'down' ? 'titlebar-output-controls' : null"
    :virtual-mic-enabled="controls.virtualMicEnabled.value"
    :vban-enabled="controls.vbanEnabled.value"
    :virtual-mic-available="controls.virtualMicAvailable.value"
    :vban-available="controls.vbanAvailable.value"
    :compact="props.compact"
    :micro="props.micro"
    :ready="controls.audioOutputsReady.value"
    :pending="controls.outputsPending.value"
    :direction="props.direction"
    @open="controls.refreshAvailability()"
    @toggle-virtual-mic="controls.toggleVirtualMicOutput"
    @toggle-vban="controls.toggleVbanOutput"
  />
</template>

<style scoped>
/* Titlebar placement only: the group keeps its spacing from the extra-window
   buttons, and its down-facing popover needs its own local layer. The
   .app-titlebar markup precedes the main content in the DOM, so the base
   popover layer (1000) can lose to later stacking contexts in the content
   (sidebar rows, status toasts); 1200 lifts just this menu above them —
   the editor placement (up) keeps the shared base layer. */
.titlebar-output-controls {
  margin-right: 0.35rem;
}

.titlebar-output-controls :deep(.output-menu--down) {
  z-index: 1200;
}
</style>
