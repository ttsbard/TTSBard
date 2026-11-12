import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { presentCommandError } from '../ipc/commandError'
import { t } from '../i18n'
import { debugWarn } from '../utils/debug'

export const MODEL_DOWNLOADS_STATUS_COMMAND = 'model_downloads_status'
export const MODEL_DOWNLOADS_INSTALL_COMMAND = 'model_downloads_install'
export const MODEL_DOWNLOADS_CANCEL_COMMAND = 'model_downloads_cancel'
export const MODEL_DOWNLOADS_REFRESH_CATALOG_COMMAND = 'model_downloads_refresh_catalog'
export const MODEL_DOWNLOADS_OPEN_FOLDER_COMMAND = 'model_downloads_open_folder'
export const MODEL_DOWNLOADS_PROGRESS_EVENT = 'model-downloads-progress'

export const MODEL_CATEGORIES = ['ruaccent', 'ocr', 'pocket'] as const
export type ModelCategory = (typeof MODEL_CATEGORIES)[number]

export interface ModelCatalogFileDto {
  path: string
  sha256: string
  size_bytes: number
  url?: string
  content?: string
}

export interface ModelCatalogPackageDto {
  id: string
  category: ModelCategory
  name: string
  version: string
  source_url?: string
  license?: string
  recommended?: boolean
  available: boolean
  unavailable_reason?: string
  files: ModelCatalogFileDto[]
}

export interface ModelCatalogDto {
  schema_version: 1
  published_at: string
  packages: ModelCatalogPackageDto[]
}

export interface InstalledPackageDto {
  id: string
  version: string
  files_sha256?: string
  installed_at: string
}

export interface CatalogDownloadDto {
  url: string
  downloaded_at: string
}

export interface ModelDownloadsMetadataDto {
  schema_version: 1
  catalog_source_url: string | null
  installed: InstalledPackageDto[]
}

export interface ModelDownloadsStatusDto {
  catalog: ModelCatalogDto | null
  catalog_download: CatalogDownloadDto | null
  metadata: ModelDownloadsMetadataDto
  installed_package_ids: string[]
  outdated_package_ids: string[]
}

export interface ModelDownloadProgressDto {
  package_id: string
  received_bytes: number
  total_bytes: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

function isNonNegativeFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
}

function isIsoDateTime(value: unknown): value is string {
  return isNonEmptyString(value) && !Number.isNaN(Date.parse(value))
}

function parseCategory(value: unknown): ModelCategory | null {
  return typeof value === 'string' && (MODEL_CATEGORIES as readonly string[]).includes(value)
    ? (value as ModelCategory)
    : null
}

function parseCatalogFile(raw: unknown, category: ModelCategory): ModelCatalogFileDto | null {
  if (!isRecord(raw)) return null
  if (!isNonEmptyString(raw.path) || !raw.path.startsWith(`${category}/`)) return null
  if (!isNonEmptyString(raw.sha256)) return null
  if (!isNonNegativeFiniteNumber(raw.size_bytes) || !Number.isInteger(raw.size_bytes)) return null

  const hasUrl = isNonEmptyString(raw.url)
  const hasContent = typeof raw.content === 'string'
  if (hasUrl === hasContent) return null

  const file: ModelCatalogFileDto = {
    path: raw.path,
    sha256: raw.sha256,
    size_bytes: raw.size_bytes,
  }
  if (hasUrl) file.url = raw.url as string
  if (hasContent) file.content = raw.content as string
  return file
}

function parseCatalogPackage(raw: unknown): ModelCatalogPackageDto | null {
  if (!isRecord(raw)) return null
  const category = parseCategory(raw.category)
  if (!category) return null
  if (!isNonEmptyString(raw.id) || !isNonEmptyString(raw.name) || !isNonEmptyString(raw.version)) {
    return null
  }
  if (typeof raw.available !== 'boolean' || !Array.isArray(raw.files)) return null
  if (raw.source_url != null && !isNonEmptyString(raw.source_url)) return null
  if (raw.license != null && !isNonEmptyString(raw.license)) return null
  if (raw.recommended != null && typeof raw.recommended !== 'boolean') return null
  if (raw.unavailable_reason != null && !isNonEmptyString(raw.unavailable_reason)) return null

  const files: ModelCatalogFileDto[] = []
  for (const rawFile of raw.files) {
    const file = parseCatalogFile(rawFile, category)
    if (!file) return null
    files.push(file)
  }

  const pkg: ModelCatalogPackageDto = {
    id: raw.id,
    category,
    name: raw.name,
    version: raw.version,
    available: raw.available,
    files,
  }
  if (raw.source_url != null) pkg.source_url = raw.source_url
  if (raw.license != null) pkg.license = raw.license
  if (raw.recommended === true) pkg.recommended = true
  if (raw.unavailable_reason != null) pkg.unavailable_reason = raw.unavailable_reason
  return pkg
}

function parseCatalog(raw: unknown): ModelCatalogDto | null {
  if (!isRecord(raw)) return null
  if (raw.schema_version !== 1) return null
  if (!isNonEmptyString(raw.published_at) || Number.isNaN(Date.parse(raw.published_at))) return null
  if (!Array.isArray(raw.packages)) return null

  const packages: ModelCatalogPackageDto[] = []
  const ids = new Set<string>()
  for (const rawPackage of raw.packages) {
    const pkg = parseCatalogPackage(rawPackage)
    if (!pkg || ids.has(pkg.id)) return null
    ids.add(pkg.id)
    packages.push(pkg)
  }

  return {
    schema_version: 1,
    published_at: raw.published_at,
    packages,
  }
}

function emptyMetadata(): ModelDownloadsMetadataDto {
  return {
    schema_version: 1,
    catalog_source_url: null,
    installed: [],
  }
}

function parseMetadata(raw: unknown): ModelDownloadsMetadataDto | null {
  if (!isRecord(raw) || raw.schema_version !== 1) return null
  if (raw.catalog_source_url !== null && !isNonEmptyString(raw.catalog_source_url)) return null

  if (!Array.isArray(raw.installed)) return null
  const installed: InstalledPackageDto[] = []
  for (const entry of raw.installed) {
    if (!isRecord(entry)) return null
    if (!isNonEmptyString(entry.id) || !isNonEmptyString(entry.version)) return null
    if (
      entry.files_sha256 !== undefined
      && entry.files_sha256 !== null
      && !isNonEmptyString(entry.files_sha256)
    ) {
      return null
    }
    if (!isIsoDateTime(entry.installed_at)) return null
    installed.push({
      id: entry.id,
      version: entry.version,
      ...(isNonEmptyString(entry.files_sha256) ? { files_sha256: entry.files_sha256 } : {}),
      installed_at: entry.installed_at,
    })
  }

  return {
    schema_version: 1,
    catalog_source_url: raw.catalog_source_url,
    installed,
  }
}

/**
 * Strict status converter. A malformed catalog or metadata is rejected as a
 * whole instead of rendering a partially trusted package list.
 */
export function convertModelDownloadsStatus(raw: unknown): ModelDownloadsStatusDto | null {
  if (!isRecord(raw)) return null

  let catalog: ModelCatalogDto | null = null
  if (raw.catalog !== null) {
    catalog = parseCatalog(raw.catalog)
    if (!catalog) return null
  }

  let catalogDownload: CatalogDownloadDto | null = null
  if (catalog && raw.catalog_download != null) {
    const download = raw.catalog_download
    if (!isRecord(download) || !isNonEmptyString(download.url) || !isIsoDateTime(download.downloaded_at)) {
      return null
    }
    catalogDownload = { url: download.url, downloaded_at: download.downloaded_at }
  }

  let metadata = emptyMetadata()
  if (raw.metadata !== undefined && raw.metadata !== null) {
    const parsedMetadata = parseMetadata(raw.metadata)
    if (!parsedMetadata) return null
    metadata = parsedMetadata
  }

  if (!Array.isArray(raw.installed_package_ids)) return null
  if (!raw.installed_package_ids.every(isNonEmptyString)) return null

  let outdatedPackageIds: string[] = []
  if (raw.outdated_package_ids !== undefined && raw.outdated_package_ids !== null) {
    if (!Array.isArray(raw.outdated_package_ids)) return null
    if (!raw.outdated_package_ids.every(isNonEmptyString)) return null
    outdatedPackageIds = [...new Set(raw.outdated_package_ids)]
  }

  return {
    catalog,
    catalog_download: catalogDownload,
    metadata,
    installed_package_ids: [...new Set(raw.installed_package_ids)],
    outdated_package_ids: outdatedPackageIds,
  }
}

export function convertModelDownloadProgress(raw: unknown): ModelDownloadProgressDto | null {
  if (!isRecord(raw)) return null
  if (!isNonEmptyString(raw.package_id)) return null
  if (!isNonNegativeFiniteNumber(raw.received_bytes)) return null
  if (!isNonNegativeFiniteNumber(raw.total_bytes)) return null
  return {
    package_id: raw.package_id,
    received_bytes: raw.received_bytes,
    total_bytes: raw.total_bytes,
  }
}

type StatusLoadOutcome = 'applied' | 'superseded' | 'failed'

export function useModelDownloads() {
  const status = ref<ModelDownloadsStatusDto>({
    catalog: null,
    catalog_download: null,
    metadata: emptyMetadata(),
    installed_package_ids: [],
    outdated_package_ids: [],
  })
  const statusLoaded = ref(false)
  const statusLoading = ref(false)
  const statusError = ref(false)
  const progressByPackage = ref<Record<string, ModelDownloadProgressDto>>({})
  const errorByPackage = ref<Record<string, string>>({})
  const pendingInstall = ref<Record<string, boolean>>({})
  const pendingCancel = ref<Record<string, boolean>>({})
  const catalogRefreshPending = ref(false)
  const folderPending = ref(false)

  let disposed = false
  let statusGeneration = 0
  let unlistenProgress: (() => void) | null = null

  function setRecordFlag(
    target: typeof pendingInstall,
    packageId: string,
    value: boolean,
  ): void {
    if (value) {
      target.value = { ...target.value, [packageId]: true }
    } else {
      const next = { ...target.value }
      delete next[packageId]
      target.value = next
    }
  }

  function clearProgress(packageId: string): void {
    if (!(packageId in progressByPackage.value)) return
    const next = { ...progressByPackage.value }
    delete next[packageId]
    progressByPackage.value = next
  }

  function clearPackageError(packageId: string): void {
    if (!(packageId in errorByPackage.value)) return
    const next = { ...errorByPackage.value }
    delete next[packageId]
    errorByPackage.value = next
  }

  async function loadStatus(options: { silent?: boolean } = {}): Promise<StatusLoadOutcome> {
    const generation = ++statusGeneration
    if (!options.silent) statusLoading.value = true
    try {
      const payload = await invoke<unknown>(MODEL_DOWNLOADS_STATUS_COMMAND)
      if (disposed) return 'superseded'
      if (generation !== statusGeneration) return 'superseded'
      const converted = convertModelDownloadsStatus(payload)
      if (!converted) {
        debugWarn('[useModelDownloads] Ignoring malformed status response')
        statusError.value = true
        return 'failed'
      }
      status.value = converted
      statusLoaded.value = true
      statusError.value = false
      return 'applied'
    } catch (error) {
      if (disposed || generation !== statusGeneration) return 'superseded'
      debugWarn('[useModelDownloads] Failed to load status:', error)
      statusError.value = true
      return 'failed'
    } finally {
      if (!disposed && generation === statusGeneration) statusLoading.value = false
    }
  }

  async function refreshStatus(options: { silent?: boolean } = {}): Promise<boolean> {
    return (await loadStatus(options)) === 'applied'
  }

  async function installPackage(packageId: string): Promise<'installed' | 'cancelled' | null> {
    if (!packageId || pendingInstall.value[packageId]) return null
    clearPackageError(packageId)
    clearProgress(packageId)
    progressByPackage.value = {
      ...progressByPackage.value,
      [packageId]: {
        package_id: packageId,
        received_bytes: 0,
        total_bytes: 0,
      },
    }
    setRecordFlag(pendingInstall, packageId, true)
    try {
      const outcome = await invoke<unknown>(MODEL_DOWNLOADS_INSTALL_COMMAND, { packageId })
      if (disposed) return null
      if (outcome !== 'installed' && outcome !== 'cancelled') {
        throw new Error('Invalid model install outcome')
      }
      clearProgress(packageId)
      await refreshStatus({ silent: true })
      return outcome
    } catch (error) {
      if (disposed) return null
      clearProgress(packageId)
      errorByPackage.value = {
        ...errorByPackage.value,
        [packageId]: presentCommandError(error, t('settings.models.error.install')),
      }
      return null
    } finally {
      if (!disposed) {
        clearProgress(packageId)
        setRecordFlag(pendingCancel, packageId, false)
        setRecordFlag(pendingInstall, packageId, false)
      }
    }
  }

  async function cancelPackage(packageId: string): Promise<boolean> {
    if (!pendingInstall.value[packageId] || pendingCancel.value[packageId]) return false
    setRecordFlag(pendingCancel, packageId, true)
    clearPackageError(packageId)
    try {
      await invoke(MODEL_DOWNLOADS_CANCEL_COMMAND, { packageId })
      return !disposed
    } catch (error) {
      if (disposed || !pendingInstall.value[packageId]) return false
      setRecordFlag(pendingCancel, packageId, false)
      errorByPackage.value = {
        ...errorByPackage.value,
        [packageId]: presentCommandError(error, t('settings.models.error.cancel')),
      }
      return false
    }
  }

  async function refreshCatalog(): Promise<boolean> {
    if (catalogRefreshPending.value) return false
    catalogRefreshPending.value = true
    try {
      await invoke(MODEL_DOWNLOADS_REFRESH_CATALOG_COMMAND)
      if (disposed) return false
      const outcome = await loadStatus({ silent: true })
      if (disposed) return false
      // The command has already committed the catalog. A newer status read may
      // supersede ours, but that read will render the saved catalog, so only a
      // real status failure makes this catalog update unsuccessful.
      return outcome !== 'failed'
    } catch (error) {
      if (disposed) return false
      debugWarn('[useModelDownloads] Failed to refresh catalog:', error)
      return false
    } finally {
      if (!disposed) catalogRefreshPending.value = false
    }
  }

  async function openFolder(): Promise<boolean> {
    if (folderPending.value) return false
    folderPending.value = true
    try {
      await invoke(MODEL_DOWNLOADS_OPEN_FOLDER_COMMAND)
      return !disposed
    } catch (error) {
      if (!disposed) debugWarn('[useModelDownloads] Failed to open models folder:', error)
      return false
    } finally {
      if (!disposed) folderPending.value = false
    }
  }

  async function registerProgressListener(): Promise<void> {
    try {
      const unlisten = await listen<unknown>(MODEL_DOWNLOADS_PROGRESS_EVENT, (event) => {
        if (disposed) return
        const progress = convertModelDownloadProgress(event.payload)
        if (!progress) {
          debugWarn('[useModelDownloads] Ignoring malformed progress event')
          return
        }
        if (!pendingInstall.value[progress.package_id] || pendingCancel.value[progress.package_id]) return
        progressByPackage.value = {
          ...progressByPackage.value,
          [progress.package_id]: progress,
        }
      })
      if (disposed) {
        unlisten()
        return
      }
      unlistenProgress = unlisten
    } catch (error) {
      debugWarn('[useModelDownloads] Failed to register progress listener:', error)
    }
  }

  onMounted(() => {
    void registerProgressListener()
  })

  onUnmounted(() => {
    disposed = true
    statusGeneration++
    unlistenProgress?.()
    unlistenProgress = null
  })

  return {
    status,
    statusLoaded,
    statusLoading,
    statusError,
    progressByPackage,
    errorByPackage,
    pendingInstall,
    pendingCancel,
    catalogRefreshPending,
    folderPending,
    refreshStatus,
    installPackage,
    cancelPackage,
    refreshCatalog,
    openFolder,
  }
}
