// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { createVirtualMicDeviceCheck } from './useVirtualMicDevices'

type DeviceList = Array<{ id: string; name: string; is_default: boolean }>

const devices = (ids: string[]): DeviceList =>
  ids.map((id) => ({ id, name: `Device ${id}`, is_default: false }))

describe('virtual mic device availability check (ROADMAP-142 polish)', () => {
  it('answers false before the list is known and for empty ids', async () => {
    const invokeFn = vi.fn(async () => devices(['A', 'B']))
    const check = createVirtualMicDeviceCheck(invokeFn)

    expect(check.deviceIds.value).toBeNull()
    expect(check.isSelectedDeviceAvailable('A')).toBe(false) // unknown list
    expect(check.isSelectedDeviceAvailable(null)).toBe(false)
    expect(check.isSelectedDeviceAvailable('')).toBe(false)

    await check.refresh()
    expect(check.isSelectedDeviceAvailable('A')).toBe(true)
    expect(check.isSelectedDeviceAvailable('C')).toBe(false)
  })

  it('a selected device that disappeared from the list is unavailable', async () => {
    const invokeFn = vi.fn(async () => devices(['A']))
    const check = createVirtualMicDeviceCheck(invokeFn)
    await check.refresh()

    expect(check.isSelectedDeviceAvailable('A')).toBe(true)
    // The saved id is non-empty, but the hardware is gone (unplugged VB-Cable).
    expect(check.isSelectedDeviceAvailable('B')).toBe(false)
  })

  it('a stale response never overwrites a fresher one', async () => {
    const invokeFn = vi.fn(
      (_cmd: string) =>
        new Promise<DeviceList>((resolve) => {
          queueMicrotask(() => resolve(devices(['first'])))
        }),
    )
    const check = createVirtualMicDeviceCheck(invokeFn)

    // Start the older refresh first; its response lands after a newer one.
    const older = check.refresh()
    await older
    expect(check.isSelectedDeviceAvailable('first')).toBe(true)

    // A genuinely overlapping case: the first response lands last.
    const slow: Array<(list: DeviceList) => void> = []
    const fn = vi.fn(
      (_cmd: string) =>
        new Promise<DeviceList>((resolve) => {
          slow.push(resolve)
        }),
    )
    const racing = createVirtualMicDeviceCheck(fn)
    const first = racing.refresh()
    const second = racing.refresh()
    slow[1]!(devices(['fresh'])) // the newer refresh resolves first
    slow[0]!(devices(['stale'])) // the older one lands afterwards
    await Promise.all([first, second])

    expect(racing.isSelectedDeviceAvailable('fresh')).toBe(true)
    expect(racing.isSelectedDeviceAvailable('stale')).toBe(false)
  })

  it('a deferred refresh keeps the last confirmed list until its own response', async () => {
    // Первый success: устройство доступно.
    const resolvers: Array<(list: DeviceList) => void> = []
    const rejecters: Array<(reason: unknown) => void> = []
    let failNext = false
    const fn = vi.fn((_cmd: string) => {
      if (failNext) {
        return new Promise<DeviceList>((_resolve, reject) => rejecters.push(reject))
      }
      return new Promise<DeviceList>((resolve) => resolvers.push(resolve))
    })
    const check = createVirtualMicDeviceCheck(fn)

    const first = check.refresh()
    resolvers[0]!(devices(['A']))
    await first
    expect(check.isSelectedDeviceAvailable('A')).toBe(true)

    // Повторный (deferred) refresh НЕ обнуляет последнее подтверждённое
    // состояние: доступность сохраняется до ответа, без мигания
    // available→disabled→available при каждом открытии меню.
    const second = check.refresh()
    expect(check.isSelectedDeviceAvailable('A')).toBe(true)

    // Свежий ответ без устройства: 'A' становится unavailable.
    resolvers[1]!(devices(['B']))
    await second
    expect(check.isSelectedDeviceAvailable('A')).toBe(false)
    expect(check.isSelectedDeviceAvailable('B')).toBe(true)

    // Ошибка свежего запроса делает состояние unavailable.
    failNext = true
    const third = check.refresh()
    expect(check.isSelectedDeviceAvailable('A')).toBe(false)
    expect(check.isSelectedDeviceAvailable('B')).toBe(true)
    rejecters[0]!(new Error('device enumeration failed'))
    await third
    expect(check.isSelectedDeviceAvailable('B')).toBe(false)
    expect(check.deviceIds.value).toBeNull()
  })

  it('an IPC failure leaves the list unknown (no false availability) and a retry recovers', async () => {
    let fail = true
    const invokeFn = vi.fn(async () => {
      if (fail) throw new Error('device enumeration failed')
      return devices(['A'])
    })
    const check = createVirtualMicDeviceCheck(invokeFn)

    await check.refresh()
    expect(check.deviceIds.value).toBeNull()
    expect(check.isSelectedDeviceAvailable('A')).toBe(false)

    fail = false // the next attempt (e.g. the next menu open) retries
    await check.refresh()
    expect(check.isSelectedDeviceAvailable('A')).toBe(true)
  })
})
