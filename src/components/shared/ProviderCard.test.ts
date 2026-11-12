// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import type { Component } from 'vue'
import { Cpu } from 'lucide-vue-next'
import ProviderCard from './ProviderCard.vue'
import { restoreProviderRadioState } from './providerCardState'

const mounted: Array<{ unmount(): void }> = []

interface CardProps {
  title: string
  icon?: Component
  active?: boolean
  expanded?: boolean
  disabled?: boolean
  collapsible?: boolean
  selectable?: boolean
  onSelect?: () => void
  onToggle?: () => void
}

async function mountCard(
  props: CardProps,
  slots: Record<string, () => any> = {},
) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(ProviderCard, props, slots),
  })
  app.mount(host)
  mounted.push({
    unmount() {
      app.unmount()
      host.remove()
    },
  })
  await nextTick()
  return host
}

afterEach(() => {
  for (const item of mounted.splice(0)) item.unmount()
})

describe('ProviderCard radio state', () => {
  it('restores an inactive provider after the browser checks its radio', () => {
    const radio = { checked: true }

    restoreProviderRadioState(radio, false)

    expect(radio.checked).toBe(false)
  })

  it('keeps an active provider checked', () => {
    const radio = { checked: false }

    restoreProviderRadioState(radio, true)

    expect(radio.checked).toBe(true)
  })
})

describe('ProviderCard layout', () => {
  it('keeps the interactive collapsible header by default', async () => {
    const select = vi.fn()
    const toggle = vi.fn()
    const host = await mountCard({
      title: 'Test provider',
      icon: Cpu,
      onSelect: select,
      onToggle: toggle,
    })

    expect(host.querySelector('.card-header input[type="radio"]')).not.toBeNull()
    expect(host.querySelector('.expand-icon')).not.toBeNull()
    expect(host.querySelector('.card-content')).toBeNull()
    expect(host.querySelector('.provider-icon.lucide-cpu')).not.toBeNull()
  })

  it('renders an always-expanded, non-selectable card without an empty radio indent', async () => {
    const host = await mountCard(
      {
        title: 'Static provider',
        icon: Cpu,
        collapsible: false,
        selectable: false,
      },
      { default: () => h('div', { class: 'card-body' }, 'Body') },
    )

    expect(host.querySelector('.card-header input')).toBeNull()
    expect(host.querySelector('.expand-icon')).toBeNull()
    expect(host.querySelector('.card-content .card-body')?.textContent).toBe('Body')
    expect(host.querySelector('.card-header')?.firstElementChild?.classList.contains('provider-icon')).toBe(true)
    expect(host.querySelector('.provider-card')?.classList.contains('provider-card--static')).toBe(true)
  })

  it('renders the header-actions slot separately from the expand icon', async () => {
    const toggle = vi.fn()
    const host = await mountCard(
      { title: 'Action card', icon: Cpu, expanded: true, onToggle: toggle },
      {
        default: () => h('div', 'Body'),
        'header-actions': () => h('button', { class: 'header-action' }, 'Refresh'),
      },
    )

    const action = host.querySelector('.header-actions .header-action') as HTMLButtonElement
    expect(action).not.toBeNull()
    expect(host.querySelector('.expand-icon')).not.toBeNull()

    action.click()
    await nextTick()
    expect(toggle).not.toHaveBeenCalled()
  })
})
