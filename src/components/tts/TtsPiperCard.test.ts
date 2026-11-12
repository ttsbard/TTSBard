// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import type { TtsProviderInfoDto } from '../../types/settings'
import TtsPiperCard from './TtsPiperCard.vue'

const mounted: Array<{ unmount(): void }> = []

function piper(
  id: string,
  displayName: string,
  overrides: Partial<TtsProviderInfoDto> = {},
): TtsProviderInfoDto {
  return {
    id: `local-piper:${id}`,
    display_name: displayName,
    kind: 'piper',
    active: false,
    runtime_status: 'ready',
    ...overrides,
  }
}

async function flush() {
  await Promise.resolve()
  await nextTick()
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}

async function mountCard(props: {
  providers: TtsProviderInfoDto[]
  activeId?: string | null
  loading?: Record<string, boolean>
  errors?: Record<string, string | null>
}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const select = vi.fn()
  const app = createApp({
    render: () => h(TtsPiperCard, {
      ...props,
      onSelect: select,
    }),
  })
  app.mount(host)
  mounted.push({
    unmount() {
      app.unmount()
      host.remove()
    },
  })
  await flush()
  return { host, select }
}

describe('TtsPiperCard', () => {
  afterEach(() => {
    for (const item of mounted.splice(0)) item.unmount()
  })

  it('renders every Piper model inside an always-expanded, non-selectable card with a Cpu icon', async () => {
    const first = piper('amy', 'Amy')
    const second = piper('giga', 'Giga')
    const { host } = await mountCard({ providers: [first, second] })

    expect(host.querySelector('.provider-card')).not.toBeNull()
    expect(host.querySelector('.card-content')).not.toBeNull()
    expect(host.querySelector('.expand-icon')).toBeNull()
    expect(host.querySelector('.card-header input[type="radio"]')).toBeNull()
    expect(host.querySelector('.provider-icon.lucide-cpu')).not.toBeNull()
    expect(host.querySelectorAll('.local-model-row')).toHaveLength(2)
    expect(host.textContent).toContain('Amy')
    expect(host.textContent).toContain('Giga')
    expect(host.textContent).toContain('Ready')
  })

  it('highlights the active card and the active model row', async () => {
    const first = piper('amy', 'Amy')
    const second = piper('giga', 'Giga')
    const { host } = await mountCard({ providers: [first, second], activeId: second.id })

    const card = host.querySelector('.provider-card')
    expect(card?.classList.contains('active')).toBe(true)
    const rows = [...host.querySelectorAll('.local-model-row')]
    expect(rows[0].classList.contains('local-model-row--active')).toBe(false)
    expect(rows[1].classList.contains('local-model-row--active')).toBe(true)
    const radios = host.querySelectorAll<HTMLInputElement>('input[name="piper-provider"]')
    expect(radios[1].checked).toBe(true)
  })

  it('emits the model id when its radio changes', async () => {
    const first = piper('amy', 'Amy')
    const second = piper('giga', 'Giga')
    const { host, select } = await mountCard({ providers: [first, second] })

    host.querySelectorAll<HTMLInputElement>('input[name="piper-provider"]')[1].click()
    await flush()

    expect(select).toHaveBeenCalledTimes(1)
    expect(select).toHaveBeenCalledWith(second.id)
  })

  it('shows loading, ready and per-model error states inline', async () => {
    const ready = piper('amy', 'Amy')
    const loading = piper('giga', 'Giga')
    const failed = piper('irina', 'Irina')
    const errorText = 'Failed to select the Piper model'
    const { host } = await mountCard({
      providers: [ready, loading, failed],
      loading: { [loading.id]: true },
      errors: { [failed.id]: errorText },
    })

    const rows = [...host.querySelectorAll('.local-model-row')]
    expect(rows[0].textContent).toContain('Ready')
    expect(rows[0].querySelector('.local-model-status--ready')).not.toBeNull()
    expect(rows[1].textContent).toContain('Loading…')
    expect(rows[1].querySelector('.local-model-status--loading')).not.toBeNull()
    expect(rows[1].querySelector('.ui-spin')).not.toBeNull()
    expect(rows[2].textContent).toContain('Loading error')
    expect(rows[2].querySelector('.local-model-status--error')).not.toBeNull()

    const message = host.querySelector('.local-model-message--error')
    expect(message).not.toBeNull()
    expect(message?.getAttribute('role')).toBe('alert')
    expect(message?.textContent).toContain(errorText)
  })

  it('does not add an empty indent where the header radio would be', async () => {
    const { host } = await mountCard({ providers: [piper('amy', 'Amy')] })

    const header = host.querySelector('.card-header')!
    expect(header.querySelector('input')).toBeNull()
    expect(header.firstElementChild?.classList.contains('provider-icon')).toBe(true)
  })
})
