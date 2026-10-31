// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import TelegramConnectionStatus from './TelegramConnectionStatus.vue'

const apps: Array<{ unmount(): void }> = []

function mountStatus(props: Record<string, unknown> = {}, emits: Record<string, (...args: unknown[]) => void> = {}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render() {
      return h(TelegramConnectionStatus, {
        ...props,
        ...emits,
      } as any)
    },
  })
  app.mount(host)
  apps.push({
    unmount() {
      app.unmount()
      host.remove()
    },
  })
  return { host }
}

describe('TelegramConnectionStatus timeout settings (ROADMAP-133)', () => {
  afterEach(() => {
    for (const app of apps.splice(0)) app.unmount()
  })

  it('renders the gear button always even when disconnected', () => {
    const { host } = mountStatus({ connected: false })
    const gearBtn = host.querySelector('.timeout-gear-button') as HTMLButtonElement
    expect(gearBtn).not.toBeNull()
    expect(gearBtn.getAttribute('aria-expanded')).toBe('false')
    expect(host.querySelector('.timeout-settings-row')).toBeNull()
  })

  it('toggles the timeout settings drawer on gear button click', async () => {
    const { host } = mountStatus({ connected: true, synthesisResponseTimeoutMs: 60000 })
    const gearBtn = host.querySelector('.timeout-gear-button') as HTMLButtonElement
    expect(gearBtn).not.toBeNull()

    // Initially closed
    expect(gearBtn.getAttribute('aria-expanded')).toBe('false')
    expect(host.querySelector('.timeout-settings-row')).toBeNull()

    // Click to open
    gearBtn.click()
    await nextTick()

    expect(gearBtn.getAttribute('aria-expanded')).toBe('true')
    const drawer = host.querySelector('.timeout-settings-row')
    expect(drawer).not.toBeNull()
    const input = drawer?.querySelector('#silero-timeout-input') as HTMLInputElement
    expect(input).not.toBeNull()
    expect(input.value).toBe('60')

    // Click to close
    gearBtn.click()
    await nextTick()
    expect(host.querySelector('.timeout-settings-row')).toBeNull()
  })

  it('emits timeout-change with milliseconds when valid seconds entered and changed', async () => {
    const onTimeoutChange = vi.fn()
    const { host } = mountStatus(
      { connected: false, synthesisResponseTimeoutMs: 60000 },
      { 'onTimeout-change': onTimeoutChange }
    )

    const gearBtn = host.querySelector('.timeout-gear-button') as HTMLButtonElement
    gearBtn.click()
    await nextTick()

    const input = host.querySelector('#silero-timeout-input') as HTMLInputElement
    expect(input).not.toBeNull()

    input.value = '45'
    input.dispatchEvent(new Event('change'))
    await nextTick()

    expect(onTimeoutChange).toHaveBeenCalledWith(45000)
  })

  it('reverts to current valid value and does not emit when invalid number entered', async () => {
    const onTimeoutChange = vi.fn()
    const { host } = mountStatus(
      { connected: false, synthesisResponseTimeoutMs: 60000 },
      { 'onTimeout-change': onTimeoutChange }
    )

    const gearBtn = host.querySelector('.timeout-gear-button') as HTMLButtonElement
    gearBtn.click()
    await nextTick()

    const input = host.querySelector('#silero-timeout-input') as HTMLInputElement
    input.value = '-5'
    input.dispatchEvent(new Event('change'))
    await nextTick()

    expect(onTimeoutChange).not.toHaveBeenCalled()
    expect(input.value).toBe('60')
  })
})
