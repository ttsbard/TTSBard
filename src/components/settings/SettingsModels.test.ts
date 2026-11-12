// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import SettingsModels from './SettingsModels.vue'
import { MODEL_DOWNLOADS_STATUS_COMMAND } from '../../composables/useModelDownloads'

const { openUrlMock } = vi.hoisted(() => ({ openUrlMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: openUrlMock,
}))

vi.mock('./SettingsRuAccent.vue', () => ({
  default: {
    name: 'SettingsRuAccent',
    methods: {
      refreshFromDisk: vi.fn(async () => {}),
    },
    render: () => null,
  },
}))

async function flushAsync() {
  for (let i = 0; i < 10; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

function buildStatusPayload() {
  return {
    catalog: {
      schema_version: 1,
      published_at: '2026-11-11',
      packages: [
        {
          id: 'ruaccent-tiny',
          category: 'ruaccent',
          name: 'RUAccent Tiny',
          version: '1.0',
          source_url: 'https://huggingface.co/ruaccent/accentuator',
          license: 'MIT',
          recommended: true,
          available: true,
          unavailable_reason: null,
          files: [
            {
              path: 'ruaccent/model.onnx',
              url: 'https://example.com/ruaccent.onnx',
              content: null,
              sha256: 'a'.repeat(64),
              size_bytes: 160 * 1024 * 1024,
            },
          ],
        },
        {
          id: 'ruaccent-small-poetry',
          category: 'ruaccent',
          name: 'RUAccent Small Poetry',
          version: '1.0',
          source_url: 'https://huggingface.co/ruaccent/accentuator',
          license: 'MIT',
          recommended: false,
          available: true,
          unavailable_reason: null,
          files: [
            {
              path: 'ruaccent/poetry.onnx',
              url: 'https://example.com/poetry.onnx',
              content: null,
              sha256: 'c'.repeat(64),
              size_bytes: 198 * 1024 * 1024,
            },
          ],
        },
        {
          id: 'eslav-ppocrv5-mobile',
          category: 'ocr',
          name: 'eSlav PP-OCRv5 Mobile',
          version: '5.0',
          source_url: 'https://www.modelscope.cn/models/RapidAI/RapidOCR',
          license: 'Apache-2.0',
          available: true,
          unavailable_reason: null,
          files: [
            {
              path: 'ocr/det.onnx',
              url: 'https://example.com/det.onnx',
              content: null,
              sha256: 'b'.repeat(64),
              size_bytes: 12 * 1024 * 1024,
            },
          ],
        },
        {
          id: 'kyutai-en-full',
          category: 'pocket',
          name: 'Kyutai English (Alba)',
          version: '2026-10',
          source_url: 'https://huggingface.co/kyutai/pocket-tts-without-voice-cloning',
          license: 'CC-BY-4.0',
          available: false,
          unavailable_reason: 'The ONNX set tested by TTSBard is not published.',
          files: [],
        },
        {
          id: 'pocket-en',
          category: 'pocket',
          name: 'Pocket English (third-party ONNX)',
          version: '2026-04',
          source_url: 'https://huggingface.co/KevinAHM/pocket-tts-onnx',
          license: 'CC-BY-4.0',
          available: false,
          unavailable_reason: 'Incompatible ONNX layout.',
          files: [],
        },
      ],
    },
    catalog_download: null,
    metadata: {
      schema_version: 1,
      catalog_source_url: null,
      installed: [
        {
          id: 'ruaccent-tiny',
          version: '1.0',
          installed_at: '2026-11-11T10:00:00Z',
        },
      ],
    },
    installed_package_ids: ['ruaccent-tiny', 'kyutai-en-full'],
    outdated_package_ids: [] as string[],
  }
}

describe('SettingsModels', () => {
  let host: HTMLDivElement | null = null
  let app: ReturnType<typeof createApp> | null = null

  afterEach(() => {
    app?.unmount()
    app = null
    host?.remove()
    host = null
    vi.clearAllMocks()
  })

  async function mountPanel(statusOverride?: Record<string, unknown>) {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === MODEL_DOWNLOADS_STATUS_COMMAND) return statusOverride ?? buildStatusPayload()
      return undefined
    })
    openUrlMock.mockResolvedValue(undefined)

    host = document.createElement('div')
    document.body.appendChild(host)
    app = createApp(SettingsModels, { active: true })
    app.mount(host)
    await flushAsync()
    return host
  }

  it('renders compact package rows, metadata separators, and translated actions without raw keys', async () => {
    const root = await mountPanel()

    expect(root.textContent).not.toContain('settings.models.')

    const openFolderBtn = Array.from(root.querySelectorAll('button')).find((btn) =>
      btn.textContent?.includes('Open folder'),
    )
    expect(openFolderBtn?.getAttribute('title')).toContain('Models added manually')

    const ruaccentRow = root.querySelector('[data-package-id="ruaccent-tiny"]')
    expect(ruaccentRow).not.toBeNull()
    expect(ruaccentRow?.textContent).toContain('v1.0')
    expect(ruaccentRow?.textContent).toContain('160 MB')
    expect(ruaccentRow?.textContent).toContain('On disk')
    expect(ruaccentRow?.textContent).toContain('Installed ')
    expect(ruaccentRow?.querySelector('.package-actions button')?.textContent).toContain('Reinstall')

    const ocrRow = root.querySelector('[data-package-id="eslav-ppocrv5-mobile"]')
    expect(ocrRow?.textContent).toContain('Available')
    expect(ocrRow?.querySelector('.package-actions button')?.textContent).toContain('Download')

    const kyutaiRow = root.querySelector('[data-package-id="kyutai-en-full"]')
    expect(kyutaiRow?.textContent).toContain('v2026-10')
    expect(kyutaiRow?.textContent).toContain('On disk')
    expect(kyutaiRow?.querySelector('.package-actions')).toBeNull()
    const kyutaiDetailsBtn = kyutaiRow?.querySelector('.package-details-button')
    expect(kyutaiDetailsBtn?.classList.contains('ui-button--compact')).toBe(true)
    expect(kyutaiDetailsBtn?.textContent).toContain('Details')

    const pocketEnRow = root.querySelector('[data-package-id="pocket-en"]')
    expect(pocketEnRow?.textContent).toContain('Unavailable')

    expect(ruaccentRow?.textContent).toContain('Recommended')
    expect(root.querySelector('[data-package-id="ruaccent-small-poetry"]')).toBeNull()

    const ruaccentGroup = root.querySelector('[aria-labelledby="models-group-ruaccent"]')!
    const moreBtn = ruaccentGroup.querySelector<HTMLButtonElement>('.extras-toggle')!
    expect(moreBtn.getAttribute('aria-expanded')).toBe('false')
    expect(moreBtn.textContent).toContain('More models (1)')
    moreBtn.click()
    await nextTick()

    expect(moreBtn.getAttribute('aria-expanded')).toBe('true')
    expect(root.querySelector('[data-package-id="ruaccent-small-poetry"]')).not.toBeNull()
  })

  it('expands details with source repository link, license, and Download unavailable heading', async () => {
    const root = await mountPanel()

    const ruaccentRow = root.querySelector('[data-package-id="ruaccent-tiny"]')!
    const ruaccentToggle = ruaccentRow.querySelector<HTMLButtonElement>('.package-details-toggle')!
    expect(ruaccentToggle.getAttribute('aria-expanded')).toBe('false')
    ruaccentToggle.click()
    await nextTick()

    expect(ruaccentToggle.getAttribute('aria-expanded')).toBe('true')
    const ruaccentDetails = ruaccentRow.querySelector('#model-details-ruaccent-tiny')!
    expect(ruaccentDetails.textContent).toContain('License: MIT')
    const ruaccentSource = ruaccentDetails.querySelector<HTMLAnchorElement>('.package-source')!
    expect(ruaccentSource.textContent).toContain('Open model page')
    ruaccentSource.click()
    await flushAsync()
    expect(openUrlMock).toHaveBeenCalledWith('https://huggingface.co/ruaccent/accentuator')

    const kyutaiRow = root.querySelector('[data-package-id="kyutai-en-full"]')!
    const kyutaiDetailsBtn = kyutaiRow.querySelector<HTMLButtonElement>('.package-details-button')!
    kyutaiDetailsBtn.click()
    await nextTick()

    const kyutaiDetails = kyutaiRow.querySelector('#model-details-kyutai-en-full')!
    expect(kyutaiDetails.textContent).toContain('License: CC-BY-4.0')
    expect(kyutaiDetails.textContent).toContain('Download unavailable')
    expect(kyutaiDetails.textContent).toContain('The ONNX set tested by TTSBard is not published.')
  })

  it('always shows the Download catalog from repository button and empty callout without catalog', async () => {
    const emptyPayload = {
      ...buildStatusPayload(),
      catalog: null,
      metadata: {
        schema_version: 1,
        catalog_source_url: null,
        installed: [],
      },
      installed_package_ids: [],
      outdated_package_ids: [] as string[],
    }
    const root = await mountPanel(emptyPayload)

    const downloadBtn = root.querySelector<HTMLButtonElement>('[data-testid="download-catalog-button"]')
    expect(downloadBtn).not.toBeNull()
    expect(downloadBtn?.textContent).toContain('Download catalog from repository')
    expect(root.querySelectorAll('.models-group')).toHaveLength(0)

    const emptyCallout = root.querySelector('[data-testid="empty-catalog-callout"]')
    expect(emptyCallout).not.toBeNull()
    expect(emptyCallout?.textContent).toContain('has not been downloaded yet')
    expect(emptyCallout?.querySelector('button')?.textContent).toContain('Download catalog from repository')
  })

  it('highlights outdated installed packages with mismatch warning and keeps Reinstall button', async () => {
    const payload = buildStatusPayload()
    payload.catalog.packages[0].version = '2.0'
    payload.outdated_package_ids = ['ruaccent-tiny']

    const root = await mountPanel(payload)
    const ruaccentRow = root.querySelector('[data-package-id="ruaccent-tiny"]')!

    expect(ruaccentRow.textContent).toContain('Reinstall recommended')
    expect(ruaccentRow.textContent).toContain('Installed v1.0')
    expect(ruaccentRow.textContent).toContain('Version or hash mismatch')
    expect(ruaccentRow.querySelector('.package-actions button')?.textContent).toContain('Reinstall')
  })

  it('shows only the published catalog line and moves download timestamp to its title tooltip', async () => {
    const payload = {
      ...buildStatusPayload(),
      catalog_download: {
        url: 'https://example.com/catalog.json',
        downloaded_at: '2026-11-12T08:30:00Z',
      },
    }
    const root = await mountPanel(payload)
    const meta = root.querySelector('.catalog-meta')
    const label = meta?.querySelector('span')

    expect(meta?.textContent).toContain('Catalog from')
    expect(meta?.textContent).not.toContain('Catalog fetched')
    expect(label?.getAttribute('title')).toContain('Catalog fetched')
  })
})
