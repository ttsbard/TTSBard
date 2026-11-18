// @vitest-environment jsdom
import { ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { createOutputToggle, isVbanTargetConfigured } from './useOutputToggles'
import type { VbanSettingsDto } from '../types/settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const mockedInvoke = vi.mocked(invoke)

/** Deferred call factory for the snapshot/response order regressions. */
function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

describe('createOutputToggle pending contract (ROADMAP-142)', () => {
  let saved: { value: boolean | undefined }
  let reloadCalls: number
  let errors: unknown[]

  beforeEach(() => {
    saved = { value: false }
    reloadCalls = 0
    errors = []
    mockedInvoke.mockReset()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function make() {
    return createOutputToggle({
      read: () => saved.value,
      command: 'set_virtual_mic_enabled',
      reload: async () => {
        reloadCalls++
      },
      isLoading: ref(false),
      onError: (error) => errors.push(error),
    })
  }

  it('ignores toggles before the first snapshot and reports not ready', async () => {
    saved.value = undefined
    const toggle = make()
    expect(toggle.ready.value).toBe(false)
    await toggle.toggle()
    expect(mockedInvoke).not.toHaveBeenCalled()
    expect(toggle.pending.value).toBe(false)
  })

  it('releases pending after the response lands before the snapshot (retry works)', async () => {
    // The command persists and resolves while the confirming snapshot has not
    // been read yet; the explicit reload then brings the fresh value.
    const command = deferred<void>()
    mockedInvoke.mockImplementation(() => command.promise)
    const toggle = make()

    const first = toggle.toggle()
    expect(toggle.pending.value).toBe(true)
    command.resolve()
    await first
    // Persist-first: the reload runs after the resolved command.
    expect(reloadCalls).toBe(1)

    // The snapshot refresh finished: the new value is visible and a second
    // toggle flips the saved state back — the guard must not be stuck.
    expect(toggle.pending.value).toBe(false)
    saved.value = true
    mockedInvoke.mockResolvedValue(undefined)
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, 'set_virtual_mic_enabled', { enabled: false })
    expect(toggle.pending.value).toBe(false)
  })

  it('releases pending when the snapshot lands before the response', async () => {
    const command = deferred<void>()
    mockedInvoke.mockImplementation(() => command.promise)
    const toggle = make()

    const first = toggle.toggle()
    expect(toggle.pending.value).toBe(true)

    // The settings-changed reload wins the race and the shared snapshot now
    // carries the new value before the command promise resolves.
    saved.value = true
    command.resolve()
    await first

    expect(reloadCalls).toBe(1) // explicit refresh still runs and coalesces
    expect(toggle.pending.value).toBe(false)
    expect(toggle.enabled.value).toBe(true)

    // A stale snapshot value must not flip the direction of the next toggle:
    // read() is fresh, so this toggles true -> false.
    mockedInvoke.mockResolvedValue(undefined)
    saved.value = true
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, 'set_virtual_mic_enabled', { enabled: false })
  })

  it('drops concurrent toggles while one request is in flight', async () => {
    const command = deferred<void>()
    mockedInvoke.mockImplementation(() => command.promise)
    const toggle = make()

    const first = toggle.toggle()
    const second = toggle.toggle()
    command.resolve()
    await Promise.all([first, second])
    expect(mockedInvoke).toHaveBeenCalledTimes(1)
  })

  it('surfaces a command error without rollback and keeps the toggle usable', async () => {
    const failure = new Error('IPC failed')
    mockedInvoke.mockRejectedValue(failure)
    const toggle = make()

    await toggle.toggle()
    expect(errors).toEqual([failure])
    expect(toggle.pending.value).toBe(false)
    expect(toggle.enabled.value).toBe(false) // saved state, not a fake flip
    expect(reloadCalls).toBe(0) // nothing persisted, no refresh attempted

    // Retry after the failure works with the correct direction.
    mockedInvoke.mockResolvedValue(undefined)
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, 'set_virtual_mic_enabled', { enabled: true })
  })

  it('surfaces a reload error and still releases the guard (retry allowed)', async () => {
    // Defensive: the useAppSettings reload never rejects, but a custom one
    // failing must not lock the toggle either. The persist resolved but the
    // confirming refresh failed, so the snapshot stays stale (false).
    mockedInvoke.mockResolvedValue(undefined)
    let reloadAttempts = 0
    let failures: unknown[] = []
    const toggle = createOutputToggle({
      read: () => saved.value,
      command: 'set_virtual_mic_enabled',
      reload: async () => {
        reloadAttempts++
        throw new Error('refresh failed')
      },
      isLoading: ref(false),
      onError: (error) => failures.push(error),
    })

    await toggle.toggle()
    expect(reloadAttempts).toBe(1)
    expect(failures).toHaveLength(1)
    expect(toggle.pending.value).toBe(false)
    expect(toggle.enabled.value).toBe(false)
    // The toggle stays usable: the retry flips false -> true again.
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, 'set_virtual_mic_enabled', { enabled: true })
  })

  // Стаб реального контракта useAppSettings.load: активная загрузка
  // помечает pendingReload и возвращается СРАЗУ; follow-up стартует после
  // короткого isLoading=false в finally.
  function makeCoalescingLoader() {
    const isLoading = ref(false)
    let pendingReload = false
    let fetched: boolean | undefined = false // что вернул fetch этой загрузки
    let gateResolve: (() => void) | null = null
    const snapshot = ref<boolean | undefined>(false)

    async function load(): Promise<void> {
      if (isLoading.value) {
        pendingReload = true
        return // ← контракт дефекта: immediate return
      }
      isLoading.value = true
      const fetchedAtStart = fetched // GET уходит до чужого persist
      await new Promise<void>((resolve) => {
        gateResolve = resolve
      })
      snapshot.value = fetchedAtStart // снимок этой загрузки может быть устаревшим
      isLoading.value = false
      if (pendingReload) {
        pendingReload = false
        await load() // follow-up
      }
    }

    return {
      isLoading,
      load,
      resolveCurrentGate: () => gateResolve?.(),
      setFetched: (value: boolean) => {
        fetched = value
      },
      snapshot,
    }
  }

  it('waits out a coalesced reload before releasing the guard (stale re-toggle rejected)', async () => {
    const loader = makeCoalescingLoader()
    loader.load() // чужая загрузка уже активна
    expect(loader.isLoading.value).toBe(true)

    const toggle = createOutputToggle({
      read: () => loader.snapshot.value,
      command: 'set_virtual_mic_enabled',
      reload: () => loader.load(), // coalesced: вернётся сразу
      isLoading: loader.isLoading,
      onError: (error) => errors.push(error),
    })

    // Переключение №1: команда сохраняет true, вызов reload коалесцирует.
    const command = deferred<void>()
    mockedInvoke.mockImplementationOnce(() => command.promise)
    const first = toggle.toggle()
    command.resolve()
    loader.setFetched(true) // следующая загрузка увидит уже сохранённое true

    // Guard не снят: подтверждённого snapshot ещё нет.
    expect(toggle.pending.value).toBe(true)

    // Повторный toggle до реального обновления игнорируется — ни второго
    // invoke, ни повторной отправки прежнего enabled=true.
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenCalledTimes(1)

    // Чужая загрузка завершается устаревшим снимком (false): guard всё ещё
    // держится, потому что follow-up уже обещан коалесцией.
    loader.resolveCurrentGate()
    for (let i = 0; i < 10; i++) await Promise.resolve()
    expect(loader.snapshot.value).toBe(false)
    expect(toggle.pending.value).toBe(true)
    expect(mockedInvoke).toHaveBeenCalledTimes(1)

    // Follow-up доставляет сохранённое значение и заканчивает загрузку.
    loader.resolveCurrentGate()
    loader.resolveCurrentGate()
    for (let i = 0; i < 10; i++) await Promise.resolve()
    await first
    // Follow-up доставил сохранённое значение и загрузка закончилась:
    // guard снят только теперь, состояние подтверждено.
    expect(toggle.pending.value).toBe(false)
    expect(toggle.enabled.value).toBe(true)

    // Следующий toggle отправляет ОБРАТНОЕ enabled=false, а не повтор true.
    // Его reload() реально начинает загрузку — завершаем её gate-ом стаба.
    mockedInvoke.mockResolvedValue(undefined)
    const third = toggle.toggle()
    // Даём toggle#3 начать свою загрузку: она ждёт свежий gate стаба.
    for (let i = 0; i < 10; i++) await Promise.resolve()
    loader.resolveCurrentGate()
    await third
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, 'set_virtual_mic_enabled', { enabled: false })
  })

  it('blocks an unavailable output in the shared action (popover click and hotkey path)', async () => {
    // The editor hotkey resolves to the same controller action as the
    // popover row, so this guard is what protects the hotkey too.
    // Reactive like the production wiring (computed over the device list).
    const available = ref(false)
    const toggle = createOutputToggle({
      read: () => saved.value,
      command: 'set_virtual_mic_enabled',
      reload: async () => {
        reloadCalls++
      },
      isLoading: ref(false),
      isAvailable: () => available.value,
      onError: (error) => errors.push(error),
    })

    saved.value = true // snapshot exists, saved switch is on
    await toggle.toggle()
    expect(mockedInvoke).not.toHaveBeenCalled()
    expect(errors).toEqual([])
    expect(toggle.available.value).toBe(false)
    expect(toggle.enabled.value).toBe(true)

    // The hardware shows up (device list refreshed): the output becomes
    // toggleable without any state reset.
    available.value = true
    mockedInvoke.mockResolvedValue(undefined)
    await toggle.toggle()
    expect(mockedInvoke).toHaveBeenCalledOnce()
    expect(mockedInvoke).toHaveBeenCalledWith('set_virtual_mic_enabled', { enabled: false })
  })

  it('keeps an output available while merely switched off or receiver-failed (VBAN config only)', () => {
    const configured: VbanSettingsDto = {
      enabled: false,
      address: '192.168.1.50',
      port: 6980,
      stream_name: 'TTSBard',
      volume: 100,
      ping_interval: 30,
    }
    expect(isVbanTargetConfigured(configured)).toBe(true)
    expect(isVbanTargetConfigured({ ...configured, enabled: true })).toBe(true)

    expect(isVbanTargetConfigured({ ...configured, address: '  ' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, address: '192.168.1.256' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, address: 'not-an-ip' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, address: '192.168.1' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, port: 0 })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, port: 65536 })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, stream_name: '' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, stream_name: 'x'.repeat(17) })).toBe(false)
    // Mirror of the backend validate_stream_name: printable ASCII only —
    // non-ASCII names are rejected even when short.
    expect(isVbanTargetConfigured({ ...configured, stream_name: 'поток' })).toBe(false)
    expect(isVbanTargetConfigured({ ...configured, stream_name: 'tab	here' })).toBe(false)
    // Whitespace-only stays valid: the backend allows printable ASCII spaces.
    expect(isVbanTargetConfigured({ ...configured, stream_name: '   ' })).toBe(true)
    expect(isVbanTargetConfigured(undefined)).toBe(false)
  })

  it('exposes the saved value as the visible state, not a local guess', async () => {
    const external = ref<boolean | undefined>(undefined)
    let reloads = 0
    const toggle = createOutputToggle({
      read: () => external.value,
      command: 'set_vban_enabled',
      reload: async () => {
        reloads++
        external.value = true // confirming snapshot arrives
      },
      isLoading: ref(false),
      onError: () => {},
    })
    expect(toggle.enabled.value).toBe(false) // nothing read yet
    expect(toggle.ready.value).toBe(false)
    // Snapshot arrives (saved value false), then a toggle refreshes it.
    external.value = false
    expect(toggle.ready.value).toBe(true)
    mockedInvoke.mockResolvedValue(undefined)
    await toggle.toggle()
    expect(reloads).toBe(1)
    expect(toggle.enabled.value).toBe(true)
  })
})
