import { ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { debugError } from '../utils/debug'
import { createRuntimeStatusSource } from './runtimeStatusSource'
import { convertVtsStatusFromRust } from '../utils/rustStatus'
import type { VtsRuntimeState } from '../utils/rustStatus'

export type { VtsRuntimeState } from '../utils/rustStatus'

const authenticated = ref(false)
const desiredRunning = ref(false)

// Монотонный номер refresh-запроса: ответы устаревших запросов не должны
// перезаписывать более свежие authenticated / desiredRunning.
let flagsRequest = 0

function refreshConnectionFlags() {
  const requestId = ++flagsRequest
  invoke<boolean>('get_vtube_studio_authenticated')
    .then((value) => {
      if (requestId !== flagsRequest) return
      authenticated.value = value
    })
    .catch((e) => debugError('[useVtsRuntimeStatus] Failed to load authenticated:', e))
  invoke<boolean>('get_vtube_studio_desired_running')
    .then((value) => {
      if (requestId !== flagsRequest) return
      desiredRunning.value = value
    })
    .catch((e) => debugError('[useVtsRuntimeStatus] Failed to load desired running:', e))
}

const source = createRuntimeStatusSource<VtsRuntimeState>({
  command: 'get_vtube_studio_status',
  event: 'vtube-studio-status-changed',
  convert: convertVtsStatusFromRust,
  initial: 'Disconnected',
  onApplied: refreshConnectionFlags,
})

export function useVtsRuntimeStatus(): {
  state: Ref<VtsRuntimeState>
  authenticated: Ref<boolean>
  desiredRunning: Ref<boolean>
} {
  void source.ensureInit()
  return { state: source.state, authenticated, desiredRunning }
}
