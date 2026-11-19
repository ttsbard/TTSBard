// @vitest-environment jsdom
import { createApp, h, nextTick, reactive } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import OutputToggles from './OutputToggles.vue'

interface OutputTogglesProps {
  virtualMicEnabled: boolean
  vbanEnabled: boolean
  virtualMicAvailable: boolean
  vbanAvailable: boolean
  compact: boolean
  ready: boolean
  pending: boolean
  direction?: 'up' | 'down'
}

const apps: ReturnType<typeof createApp>[] = []
afterEach(() => {
  for (const app of apps.splice(0)) app.unmount()
  document.body.innerHTML = ''
})

const currentProps = reactive<OutputTogglesProps>({
  virtualMicEnabled: true,
  vbanEnabled: false,
  virtualMicAvailable: true,
  vbanAvailable: true,
  compact: false,
  ready: true,
  pending: false,
  direction: 'up',
})

function updateProps(patch: Partial<OutputTogglesProps>) {
  Object.assign(currentProps, patch)
}

function mount(
  props: Partial<OutputTogglesProps>,
  on: Partial<{
    onToggleVirtualMic: () => void
    onToggleVban: () => void
    onOpen: () => void
  }> = {},
) {
  Object.assign(currentProps, {
    virtualMicEnabled: true,
    vbanEnabled: false,
    virtualMicAvailable: true,
    vbanAvailable: true,
    compact: false,
    ready: true,
    pending: false,
    direction: 'up',
  }, props)
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    setup: () => () =>
      h(OutputToggles, {
        virtualMicEnabled: currentProps.virtualMicEnabled,
        vbanEnabled: currentProps.vbanEnabled,
        virtualMicAvailable: currentProps.virtualMicAvailable,
        vbanAvailable: currentProps.vbanAvailable,
        compact: currentProps.compact,
        ready: currentProps.ready,
        pending: currentProps.pending,
        direction: currentProps.direction,
        onToggleVirtualMic: on.onToggleVirtualMic,
        onToggleVban: on.onToggleVban,
        onOpen: on.onOpen,
      }),
  })
  apps.push(app)
  app.mount(host)
  return host
}

const baseProps = {
  virtualMicEnabled: true,
  vbanEnabled: false,
  virtualMicAvailable: true,
  vbanAvailable: true,
  compact: false,
  ready: true,
  pending: false,
}

async function openMenu(host: HTMLElement) {
  const trigger = host.querySelector<HTMLButtonElement>('.output-toggles-btn')!
  trigger.click()
  await nextTick()
  await nextTick()
  return trigger
}

const rows = (host: HTMLElement) =>
  [...host.querySelectorAll<HTMLButtonElement>('button.output-row')]

describe('OutputToggles popover (ROADMAP-142)', () => {
  it('reports both output states in the accessible button name', () => {
    const host = mount(baseProps)
    const btn = host.querySelector('.output-toggles-btn')!
    const label = btn.getAttribute('aria-label')!
    expect(label).toMatch(/virtual microphone on/i)
    expect(label).toMatch(/VBAN off/i)
    expect(btn.getAttribute('aria-haspopup')).toBe('menu')
    expect(btn.getAttribute('aria-expanded')).toBe('false')
  })

  it('stays disabled until the settings snapshot arrives', () => {
    const host = mount({ ...baseProps, ready: false })
    const btn = host.querySelector<HTMLButtonElement>('.output-toggles-btn')!
    expect(btn.disabled).toBe(true)
    btn.click()
    return Promise.resolve().then(() => {
      expect(host.querySelector('.output-menu')).toBeNull()
    })
  })

  it('opens an upward menu with one checkbox row per output', async () => {
    const host = mount(baseProps)
    await openMenu(host)
    const menu = host.querySelector('.output-menu')!
    expect(menu.getAttribute('role')).toBe('menu')
    const items = rows(host)
    expect(items).toHaveLength(2)
    expect(items[0]!.getAttribute('role')).toBe('menuitemcheckbox')
    expect(items[0]!.getAttribute('aria-checked')).toBe('true')
    expect(items[0]!.textContent).toContain('Virtual microphone')
    expect(items[1]!.getAttribute('aria-checked')).toBe('false')
    expect(items[1]!.textContent).toContain('VBAN')
  })

  it('explains an unavailable output in title/aria, without inline hints', async () => {
    const host = mount({ ...baseProps, vbanEnabled: false, vbanAvailable: false })
    await openMenu(host)
    const items = rows(host)
    expect(items[1]!.getAttribute('aria-label')).toMatch(/not configured/i)
    expect(items[1]!.getAttribute('title')).toMatch(/not configured/i)
    // The reason lives only in title/aria-label: no inline hint element.
    expect(items[1]!.querySelector('.row-hint')).toBeNull()
    expect(host.querySelectorAll('.row-hint')).toHaveLength(0)
    expect(items[1]!.disabled).toBe(true)
  })

  it('unavailable rows are disabled, muted and never toggle', async () => {
    const mic = vi.fn()
    const host = mount({ ...baseProps, virtualMicAvailable: false }, { onToggleVirtualMic: mic })
    await openMenu(host)
    const items = rows(host)
    expect(items[0]!.disabled).toBe(true)
    items[0]!.click()
    await nextTick()
    expect(mic).not.toHaveBeenCalled()
    // checkbox carries no blue fill
    expect(items[0]!.querySelector('.row-check')!.classList.contains('is-checked')).toBe(false)
    expect(items[0]!.querySelector('.row-check')!.classList.contains('is-unavailable')).toBe(true)
  })

  it('trigger icons do not depict an unavailable output as active', () => {
    const host = mount({ ...baseProps, virtualMicAvailable: false })
    const icons = [...host.querySelectorAll('.out-icon-wrap')]
    expect(icons[0]!.classList.contains('is-off')).toBe(true) // unavailable mic
    expect(icons[1]!.classList.contains('is-off')).toBe(true) // switched off vban
  })

  it('keyboard navigation skips unavailable rows', async () => {
    const host = mount({ ...baseProps, vbanAvailable: false })
    await openMenu(host)
    const items = rows(host)
    expect(document.activeElement).toBe(items[0]) // first available gets focus
    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))
    await nextTick()
    expect(document.activeElement).toBe(items[0]) // skip the unavailable row
    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }))
    await nextTick()
    expect(document.activeElement).toBe(items[0])
  })

  it('both rows unavailable: the menu still opens to explain and closes cleanly', async () => {
    const host = mount({ ...baseProps, virtualMicAvailable: false, vbanAvailable: false })
    const trigger = host.querySelector<HTMLButtonElement>('.output-toggles-btn')!
    // A real click focuses the trigger (jsdom programmatic click does not).
    trigger.focus()
    await openMenu(host)
    const items = rows(host)
    expect(items.every((item) => item.disabled)).toBe(true)
    expect(host.querySelector('.output-menu')).not.toBeNull()
    expect(trigger.getAttribute('aria-expanded')).toBe('true')
    // No focusable row exists: focus stays on the trigger, no trap is created.
    expect(document.activeElement).toBe(trigger)

    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
    expect(document.activeElement).toBe(trigger)

    // Tab from the trigger closes the menu too and lets focus move out.
    await openMenu(host)
    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab' }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
  })

  it('opening the menu asks the owner to refresh availability', async () => {
    const open = vi.fn()
    const host = mount(baseProps, { onOpen: open })
    await openMenu(host)
    expect(open).toHaveBeenCalledOnce()
  })

  it('toggling a row emits the per-output event and keeps the menu open', async () => {
    const mic = vi.fn()
    const vban = vi.fn()
    const host = mount(baseProps, { onToggleVirtualMic: mic, onToggleVban: vban })
    await openMenu(host)

    rows(host)[1]!.click()
    await nextTick()
    expect(vban).toHaveBeenCalledOnce()
    expect(mic).not.toHaveBeenCalled()
    expect(host.querySelector('.output-menu')).not.toBeNull()

    rows(host)[0]!.click()
    await nextTick()
    expect(mic).toHaveBeenCalledOnce()
    expect(host.querySelector('.output-menu')).not.toBeNull()
  })

  it('moves real focus between rows with arrows; Home/End work too', async () => {
    const host = mount(baseProps)
    await openMenu(host)
    const items = rows(host)
    expect(document.activeElement).toBe(items[0])

    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))
    await nextTick()
    expect(document.activeElement).toBe(items[1])

    items[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }))
    await nextTick()
    expect(document.activeElement).toBe(items[1])

    items[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp' }))
    await nextTick()
    expect(document.activeElement).toBe(items[0])
  })

  it('Escape closes the menu and returns focus to the button', async () => {
    const host = mount(baseProps)
    const trigger = await openMenu(host)
    expect(host.querySelector('.output-menu')).not.toBeNull()

    rows(host)[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(document.activeElement).toBe(trigger)
  })

  it('Tab closes the menu and leaves focus to the browser', async () => {
    const host = mount(baseProps)
    const trigger = await openMenu(host)
    rows(host)[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab' }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
    // Tab intentionally does not refocus the trigger: the browser moves focus.
    expect(document.activeElement).not.toBe(trigger)
  })

  it('direction down anchors the popover below the trigger with a down chevron', async () => {
    const host = mount({ ...baseProps, direction: 'down' })
    await openMenu(host)
    const menu = host.querySelector('.output-menu')!
    expect(menu.classList.contains('output-menu--down')).toBe(true)
    expect(menu.classList.contains('output-menu--up')).toBe(false)
    // The titlebar variant points down: the chevron icon itself is flipped.
    const chevron = host.querySelector('.chevron')!
    expect(chevron.classList.contains('lucide-chevron-down')).toBe(true)
    expect(chevron.classList.contains('lucide-chevron-up')).toBe(false)
    // Default (no direction) keeps the historical upward placement.
    const upHost = mount(baseProps)
    await openMenu(upHost)
    const upMenu = upHost.querySelector('.output-menu')!
    expect(upMenu.classList.contains('output-menu--up')).toBe(true)
    expect(upMenu.classList.contains('output-menu--down')).toBe(false)
    expect(upHost.querySelector('.chevron')!.classList.contains('lucide-chevron-up')).toBe(true)
  })

  it('a click outside closes the menu', async () => {
    const host = mount(baseProps)
    await openMenu(host)
    expect(host.querySelector('.output-menu')).not.toBeNull()

    document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
  })

  it('ArrowDown from the trigger enters the menu after async availability arrives', async () => {
    const host = mount({ ...baseProps, virtualMicAvailable: false, vbanAvailable: false })
    const trigger = host.querySelector<HTMLButtonElement>('.output-toggles-btn')!
    trigger.focus()
    await openMenu(host) // both unavailable: focus stays on the trigger
    expect(document.activeElement).toBe(trigger)

    // The async device list arrives: the mic row becomes focusable.
    updateProps({ virtualMicAvailable: true })
    await nextTick()

    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))
    await nextTick()
    const items = rows(host)
    expect(document.activeElement).toBe(items[0]) // the mic row
    expect(items[1]!.disabled).toBe(true) // VBAN still unavailable

    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp' }))
    await nextTick()
    expect(document.activeElement).toBe(items[0]) // last available is the mic too

    // Escape from the row closes the menu and returns focus to the trigger.
    items[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()
    expect(document.activeElement).toBe(trigger)
  })
})

describe('OutputToggles viewport clamp (ROADMAP-143 review)', () => {
  const MENU_WIDTH = 240
  const originalInnerWidth = window.innerWidth

  /** Stub layout: trigger rect + menu width; jsdom has no layout engine. */
  function stubLayout(host: HTMLElement, rect: { left: number; width: number }) {
    const trigger = host.querySelector<HTMLElement>('.output-toggles-btn')!
    vi.spyOn(trigger, 'getBoundingClientRect').mockReturnValue({
      left: rect.left,
      right: rect.left + rect.width,
      top: 0,
      bottom: 20,
      width: rect.width,
      height: 20,
      x: rect.left,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect)
    return () => {
      const menu = host.querySelector<HTMLElement>('.output-menu')
      if (menu) Object.defineProperty(menu, 'offsetWidth', { value: MENU_WIDTH, configurable: true })
    }
  }

  afterEach(() => {
    Object.defineProperty(window, 'innerWidth', { value: originalInnerWidth, configurable: true })
  })

  function setInnerWidth(width: number) {
    Object.defineProperty(window, 'innerWidth', { value: width, configurable: true })
  }

  /** Two ticks: one for placeMenu's own nextTick, one for the style patch. */
  async function resizeAndSettle() {
    window.dispatchEvent(new Event('resize'))
    await nextTick()
    await nextTick()
  }

  it('keeps the plain right-aligned CSS when the default fits the viewport', async () => {
    const host = mount(baseProps)
    const armMenuWidth = stubLayout(host, { left: 500, width: 90 })
    await openMenu(host)
    armMenuWidth()
    await resizeAndSettle()
    const menu = host.querySelector('.output-menu')!
    expect(menu.getAttribute('style')).toBeNull()
  })

  it('clamps the under-editor menu inside the viewport on a narrow window', async () => {
    // Trigger near the left edge (the editor action bar layout): the plain
    // right: 0 alignment would push the 240px menu off the left edge.
    setInnerWidth(400)
    const host = mount(baseProps)
    const armMenuWidth = stubLayout(host, { left: 10, width: 90 })
    await openMenu(host)
    armMenuWidth()
    await resizeAndSettle()
    const menu = host.querySelector('.output-menu')!
    // ideal = 90 - 240 = -150; min = 8 - 10 = -2; max = 400 - 8 - 10 - 240 = 142
    // → clamped to -2: menu viewport-left = 10 - 2 = 8 (one gap from the edge).
    expect(menu.getAttribute('style')).toBe('left: -2px; right: auto;')
  })

  it('clamps on the right side for the titlebar (down) placement in a narrow window', async () => {
    // Trigger near the right edge of a very narrow window: even the clamped
    // menu touches only the gap — the clamp holds both edges inside.
    setInnerWidth(300)
    const host = mount({ ...baseProps, direction: 'down' })
    const armMenuWidth = stubLayout(host, { left: 280, width: 20 })
    await openMenu(host)
    armMenuWidth()
    await resizeAndSettle()
    const menu = host.querySelector('.output-menu')!
    // ideal = 20 - 240 = -220; min = 8 - 280 = -272; max = 300 - 8 - 280 - 240 = -228
    // → clamped to max (-228): right edge at 280 - 228 + 240 = 292 (one gap).
    expect(menu.getAttribute('style')).toBe('left: -228px; right: auto;')
  })

  it('re-clamps when a row label changes under the open menu (language switch)', async () => {
    setInnerWidth(400)
    const host = mount(baseProps)
    const armMenuWidth = stubLayout(host, { left: 10, width: 90 })
    await openMenu(host)
    armMenuWidth()
    await resizeAndSettle()
    expect(host.querySelector('.output-menu')!.getAttribute('style')).toBe('left: -2px; right: auto;')

    // A width change without a resize event (translated labels): the bounds
    // follow the new geometry through the rows watch.
    const menu = host.querySelector<HTMLElement>('.output-menu')!
    Object.defineProperty(menu, 'offsetWidth', { value: 380, configurable: true })
    updateProps({ virtualMicEnabled: !currentProps.virtualMicEnabled })
    await nextTick()
    await nextTick()
    await nextTick()
    // ideal = 90 - 380 = -290; min = -2; max = 400 - 8 - 10 - 380 = 2 → -2.
    expect(menu.getAttribute('style')).toBe('left: -2px; right: auto;')
  })

  it('a resize listener is not leaked after close, and a stale placement pass is dropped', async () => {
    setInnerWidth(400)
    const host = mount(baseProps)
    const armMenuWidth = stubLayout(host, { left: 10, width: 90 })
    const trigger = await openMenu(host)
    armMenuWidth()
    // Close before the placement pass resolves its nextTick: the pass must
    // not write an offset for the gone menu.
    trigger.click() // toggle closes
    await nextTick()
    await nextTick()
    expect(host.querySelector('.output-menu')).toBeNull()

    // Reopen on a wide viewport: no stale -2px from the aborted pass.
    Object.defineProperty(window, 'innerWidth', { value: 2000, configurable: true })
    await openMenu(host)
    armMenuWidth()
    await nextTick()
    await nextTick()
    // No clamp needed on a wide viewport: no inline override at all.
    expect(host.querySelector('.output-menu')!.getAttribute('style')).toBeNull()

    // After close, window resizes no longer touch anything (listener gone);
    // the unmount path removes it too, so no leak across apps.
    trigger.click()
    await nextTick()
    expect(() => window.dispatchEvent(new Event('resize'))).not.toThrow()
    await nextTick()
  })
})
