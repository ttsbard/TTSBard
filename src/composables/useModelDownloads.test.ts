// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { createApp } from 'vue'
import {
  MODEL_DOWNLOADS_REFRESH_CATALOG_COMMAND,
  MODEL_DOWNLOADS_STATUS_COMMAND,
  convertModelDownloadProgress,
  convertModelDownloadsStatus,
  useModelDownloads,
} from './useModelDownloads'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}

function statusPayload() {
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
          source_url: null,
          license: 'MIT',
          recommended: true,
          available: true,
          unavailable_reason: null,
          files: [
            {
              path: 'ruaccent/model.onnx',
              url: 'https://example.com/model.onnx',
              content: null,
              sha256: 'a'.repeat(64),
              size_bytes: 1024,
            },
          ],
        },
        {
          id: 'ocr-mobile',
          category: 'ocr',
          name: 'OCR mobile',
          version: '2.0',
          source_url: 'https://example.com/ocr',
          license: null,
          available: false,
          unavailable_reason: 'No compatible export',
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
    installed_package_ids: ['ruaccent-tiny', 'ruaccent-tiny'],
  }
}

describe('convertModelDownloadsStatus', () => {
  it('accepts the backend wire shape with null option fields and dedupes installed ids', () => {
    const status = convertModelDownloadsStatus(statusPayload())

    expect(status).not.toBeNull()
    expect(status?.catalog?.packages).toHaveLength(2)
    expect(status?.catalog?.packages[0]).toMatchObject({
      id: 'ruaccent-tiny',
      category: 'ruaccent',
      recommended: true,
      available: true,
      files: [{ path: 'ruaccent/model.onnx', sha256: 'a'.repeat(64), size_bytes: 1024 }],
    })
    expect(status?.catalog?.packages[0].source_url).toBeUndefined()
    expect(status?.catalog?.packages[1].unavailable_reason).toBe('No compatible export')
    expect(status?.installed_package_ids).toEqual(['ruaccent-tiny'])
  })

  it('accepts generated manifest files with content and no URL', () => {
    const payload = statusPayload()
    const file = payload.catalog.packages[0].files[0] as unknown as {
      path: string
      url: string | null
      content: string | null
      sha256: string
      size_bytes: number
    }
    file.path = 'ruaccent/manifest.json'
    file.url = null
    file.content = '{"schemaVersion":1}'
    file.sha256 = 'b'.repeat(64)
    file.size_bytes = 17

    const status = convertModelDownloadsStatus(payload)

    expect(status?.catalog?.packages[0].files[0].content).toBe('{"schemaVersion":1}')
    expect(status?.catalog?.packages[0].files[0].url).toBeUndefined()
  })

  it('rejects a file without a hash', () => {
    const payload = statusPayload() as unknown as {
      catalog: { packages: Array<{ files: Array<{ sha256?: string }> }> }
    }
    delete payload.catalog.packages[0].files[0].sha256

    expect(convertModelDownloadsStatus(payload)).toBeNull()
  })

  it('rejects a path outside the package category', () => {
    const payload = statusPayload()
    payload.catalog.packages[0].files[0].path = 'ocr/model.onnx'

    expect(convertModelDownloadsStatus(payload)).toBeNull()
  })

  it('rejects an unsupported catalog schema version', () => {
    const payload = statusPayload()
    payload.catalog.schema_version = 2

    expect(convertModelDownloadsStatus(payload)).toBeNull()
  })

  it('accepts an omitted optional license but rejects blank values', () => {
    for (const license of ['', '   ']) {
      const payload = statusPayload()
      payload.catalog.packages[0].license = license

      expect(convertModelDownloadsStatus(payload)).toBeNull()
    }

    const omitted = statusPayload() as unknown as {
      catalog: { packages: Array<{ license?: string }> }
    }
    delete omitted.catalog.packages[0].license

    const status = convertModelDownloadsStatus(omitted)
    expect(status).not.toBeNull()
    expect(status?.catalog?.packages[0].license).toBeUndefined()
  })

  it('rejects a malformed installed id list', () => {
    const payload = statusPayload() as unknown as { installed_package_ids: unknown[] }
    payload.installed_package_ids = ['ruaccent-tiny', 42]

    expect(convertModelDownloadsStatus(payload)).toBeNull()
  })

  it('accepts missing metadata as an empty local state', () => {
    const payload = statusPayload() as unknown as { metadata?: unknown }
    delete payload.metadata

    const status = convertModelDownloadsStatus(payload)

    expect(status?.metadata).toEqual({
      schema_version: 1,
      catalog_source_url: null,
      installed: [],
    })
  })

  it('reads the download date from the active catalog, separately from installs', () => {
    const download = { url: 'https://example.com/catalog.json', downloaded_at: '2026-11-11T10:00:00Z' }
    const status = convertModelDownloadsStatus({ ...statusPayload(), catalog_download: download })

    expect(status?.catalog_download).toEqual(download)
    expect(status?.metadata.installed).toHaveLength(1)
  })

  it('dedupes outdated package ids, preserves files_sha256, and rejects malformed outdated ids', () => {
    const payload = {
      ...statusPayload(),
      outdated_package_ids: ['ruaccent-tiny', 'ruaccent-tiny'],
    }
    ;(payload.metadata.installed[0] as Record<string, unknown>).files_sha256 = 'c'.repeat(64)

    const status = convertModelDownloadsStatus(payload)
    expect(status?.outdated_package_ids).toEqual(['ruaccent-tiny'])
    expect(status?.metadata.installed[0].files_sha256).toBe('c'.repeat(64))

    expect(convertModelDownloadsStatus({
      ...statusPayload(),
      outdated_package_ids: [123],
    })).toBeNull()
  })
})

describe('convertModelDownloadProgress', () => {
  it('accepts the backend progress event payload', () => {
    expect(convertModelDownloadProgress({
      package_id: 'ruaccent-tiny',
      received_bytes: 512,
      total_bytes: 1024,
    })).toEqual({
      package_id: 'ruaccent-tiny',
      received_bytes: 512,
      total_bytes: 1024,
    })
  })

  it('rejects unknown ids and non-finite counters', () => {
    expect(convertModelDownloadProgress({ received_bytes: 1, total_bytes: 2 })).toBeNull()
    expect(convertModelDownloadProgress({
      package_id: 'x',
      received_bytes: Number.NaN,
      total_bytes: 2,
    })).toBeNull()
    expect(convertModelDownloadProgress({
      package_id: 'x',
      received_bytes: -1,
      total_bytes: 2,
    })).toBeNull()
  })
})

describe('model installation cancellation', () => {
  it.each(['installed', 'cancelled'] as const)(
    'uses the install outcome %s after a cancellation request',
    async (outcome) => {
      let finishInstall!: (result: typeof outcome) => void
      invoke.mockImplementation((command: string) => {
        if (command === 'model_downloads_install') {
          return new Promise<typeof outcome>((resolve) => { finishInstall = resolve })
        }
        if (command === 'model_downloads_status') return Promise.resolve(statusPayload())
        return Promise.resolve()
      })
      let downloads!: ReturnType<typeof useModelDownloads>
      const app = createApp({
        setup() {
          downloads = useModelDownloads()
          return () => null
        },
      })
      app.mount(document.createElement('div'))
      try {
        const installing = downloads.installPackage('ruaccent-tiny')
        expect(await downloads.cancelPackage('ruaccent-tiny')).toBe(true)
        expect(downloads.pendingCancel.value['ruaccent-tiny']).toBe(true)
        expect(downloads.pendingInstall.value['ruaccent-tiny']).toBe(true)

        finishInstall(outcome)
        expect(await installing).toBe(outcome)
        expect(downloads.pendingCancel.value['ruaccent-tiny']).toBeUndefined()
        expect(downloads.pendingInstall.value['ruaccent-tiny']).toBeUndefined()
      } finally {
        app.unmount()
        invoke.mockReset()
      }
    },
  )
})

function mountDownloads() {
  let downloads!: ReturnType<typeof useModelDownloads>
  const app = createApp({
    setup() {
      downloads = useModelDownloads()
      return () => null
    },
  })
  app.mount(document.createElement('div'))
  return { app, downloads }
}

describe('catalog refresh status race', () => {
  it('reports the saved catalog as refreshed when a newer status read supersedes its read', async () => {
    const catalogCommand = deferred<void>()
    const statusReads: Array<ReturnType<typeof deferred<unknown>>> = []
    invoke.mockImplementation((command: string) => {
      if (command === MODEL_DOWNLOADS_REFRESH_CATALOG_COMMAND) return catalogCommand.promise
      if (command === MODEL_DOWNLOADS_STATUS_COMMAND) {
        const read = deferred<unknown>()
        statusReads.push(read)
        return read.promise
      }
      return Promise.resolve()
    })

    const { app, downloads } = mountDownloads()
    try {
      const catalogRefresh = downloads.refreshCatalog()
      await flushMicrotasks()
      expect(statusReads).toHaveLength(0)

      catalogCommand.resolve()
      await flushMicrotasks()
      expect(statusReads).toHaveLength(1)

      // A separate list refresh takes over the current status generation while
      // the catalog refresh is still waiting for its own status read.
      const listRefresh = downloads.refreshStatus()
      await flushMicrotasks()
      expect(statusReads).toHaveLength(2)

      const stale = statusPayload()
      stale.catalog.published_at = '2026-11-01'
      const current = statusPayload()
      current.catalog.published_at = '2026-11-12'

      statusReads[0].resolve(stale)
      await flushMicrotasks()
      // The superseded read must not replace the displayed state.
      expect(downloads.status.value.catalog).toBeNull()

      statusReads[1].resolve(current)
      await flushMicrotasks()

      expect(await listRefresh).toBe(true)
      expect(await catalogRefresh).toBe(true)
      expect(downloads.status.value.catalog?.published_at).toBe('2026-11-12')
    } finally {
      app.unmount()
      invoke.mockReset()
    }
  })

  it('keeps real status failures on the catalog refresh path', async () => {
    const statusRead = deferred<unknown>()
    invoke.mockImplementation((command: string) => {
      if (command === MODEL_DOWNLOADS_REFRESH_CATALOG_COMMAND) return Promise.resolve()
      if (command === MODEL_DOWNLOADS_STATUS_COMMAND) return statusRead.promise
      return Promise.resolve()
    })

    const { app, downloads } = mountDownloads()
    try {
      const catalogRefresh = downloads.refreshCatalog()
      await flushMicrotasks()

      statusRead.reject(new Error('status failed'))

      expect(await catalogRefresh).toBe(false)
      expect(downloads.statusError.value).toBe(true)
      expect(downloads.catalogRefreshPending.value).toBe(false)
    } finally {
      app.unmount()
      invoke.mockReset()
    }
  })
})
