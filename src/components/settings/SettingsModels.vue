<script setup lang="ts">
import { computed, ref, watch, type ComponentPublicInstance } from 'vue'
import {
  AlertTriangle,
  Check,
  ChevronDown,
  Download,
  ExternalLink,
  FolderOpen,
  Loader2,
  RefreshCw,
  RotateCcw,
  X,
} from 'lucide-vue-next'
import { openUrl } from '@tauri-apps/plugin-opener'
import { locale, t } from '../../i18n'
import {
  useModelDownloads,
  type ModelCatalogPackageDto,
  type ModelCategory,
  type ModelDownloadProgressDto,
} from '../../composables/useModelDownloads'
import SettingsRuAccent from './SettingsRuAccent.vue'

type MessageSeverity = 'error' | 'success' | 'warning' | 'info'

const props = defineProps<{ active?: boolean }>()

const emit = defineEmits<{
  (e: 'show-message', message: string, severity?: MessageSeverity): void
}>()

const {
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
} = useModelDownloads()

const accentorRef = ref<{ refreshFromDisk: () => Promise<void> } | null>(null)
const expandedPackages = ref(new Set<string>())
const expandedExtras = ref(new Set<ModelCategory>())
const groupElements: Partial<Record<ModelCategory, HTMLElement>> = {}

const groupDefinitions: Array<{ category: ModelCategory; labelKey: string }> = [
  { category: 'ruaccent', labelKey: 'settings.models.group.ruaccent' },
  { category: 'ocr', labelKey: 'settings.models.group.ocr' },
  { category: 'pocket', labelKey: 'settings.models.group.pocket' },
]

interface CategoryPackageSplit {
  primary: ModelCatalogPackageDto[]
  extras: ModelCatalogPackageDto[]
}

const packagesByCategory = computed<Record<ModelCategory, ModelCatalogPackageDto[]>>(() => {
  const grouped: Record<ModelCategory, ModelCatalogPackageDto[]> = {
    ruaccent: [],
    ocr: [],
    pocket: [],
  }
  for (const pkg of status.value.catalog?.packages ?? []) {
    grouped[pkg.category].push(pkg)
  }
  return grouped
})

const splitPackagesByCategory = computed<Record<ModelCategory, CategoryPackageSplit>>(() => {
  const result = {} as Record<ModelCategory, CategoryPackageSplit>
  for (const { category } of groupDefinitions) {
    const items = packagesByCategory.value[category]
    const hasRecommended = items.some((pkg) => pkg.recommended === true)
    if (!hasRecommended) {
      result[category] = { primary: items, extras: [] }
      continue
    }
    const primary: ModelCatalogPackageDto[] = []
    const extras: ModelCatalogPackageDto[] = []
    for (const pkg of items) {
      if (pkg.recommended || isInstalled(pkg.id) || isActive(pkg.id) || Boolean(errorByPackage.value[pkg.id])) {
        primary.push(pkg)
      } else {
        extras.push(pkg)
      }
    }
    result[category] = { primary, extras }
  }
  return result
})

function categorySections(category: ModelCategory): Array<{ key: 'primary' | 'extras'; items: ModelCatalogPackageDto[] }> {
  const split = splitPackagesByCategory.value[category]
  const sections: Array<{ key: 'primary' | 'extras'; items: ModelCatalogPackageDto[] }> = []
  if (split.primary.length > 0) sections.push({ key: 'primary', items: split.primary })
  if (split.extras.length > 0) sections.push({ key: 'extras', items: split.extras })
  return sections
}

function toggleGroupExtras(category: ModelCategory): void {
  if (expandedExtras.value.has(category)) expandedExtras.value.delete(category)
  else expandedExtras.value.add(category)
}

const catalogPublishedText = computed(() => {
  const publishedAt = status.value.catalog?.published_at
  if (!publishedAt) return ''
  const date = formatDate(publishedAt, true)
  return date ? t('settings.models.catalog.published', { date }) : ''
})

const catalogFetchedTitle = computed(() => {
  const fetchedAt = status.value.catalog_download?.downloaded_at
  if (!fetchedAt) return ''
  const date = formatDateTime(fetchedAt)
  return date ? t('settings.models.catalog.fetched', { date }) : ''
})

const installLocked = computed(() => Object.keys(pendingInstall.value).length > 0)

const installedIds = computed(() => new Set(status.value.installed_package_ids))
const outdatedIds = computed(() => new Set(status.value.outdated_package_ids))

watch(
  () => props.active === true,
  (isActive) => {
    if (isActive) void refreshStatus()
  },
  { immediate: true },
)

function setGroupElement(
  category: ModelCategory,
  element: Element | ComponentPublicInstance | null,
): void {
  if (element instanceof HTMLElement) groupElements[category] = element
}

function focusCategory(category: ModelCategory): void {
  const element = groupElements[category]
  if (!element) return
  if (typeof element.scrollIntoView === 'function') {
    element.scrollIntoView({ block: 'nearest' })
  }
  element.focus({ preventScroll: true })
}

defineExpose({ focusCategory })

function normalizeDate(value: string): Date | null {
  const dateOnly = /^\d{4}-\d{2}-\d{2}$/.test(value)
  const date = new Date(dateOnly ? `${value}T00:00:00Z` : value)
  return Number.isNaN(date.getTime()) ? null : date
}

function formatDate(value: string, utc = false): string {
  const date = normalizeDate(value)
  if (!date) return ''
  const options: Intl.DateTimeFormatOptions = {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }
  if (utc) options.timeZone = 'UTC'
  return new Intl.DateTimeFormat(locale.value, options).format(date)
}

function formatDateTime(value: string): string {
  const date = normalizeDate(value)
  if (!date) return value
  return new Intl.DateTimeFormat(locale.value, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date)
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return ''
  if (bytes < 1024) {
    return `${new Intl.NumberFormat(locale.value, { maximumFractionDigits: 0 }).format(bytes)} B`
  }
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const digits = value >= 10 ? 0 : 1
  const formatted = new Intl.NumberFormat(locale.value, {
    maximumFractionDigits: digits,
  }).format(value)
  return `${formatted} ${units[unitIndex]}`
}

function packageSize(pkg: ModelCatalogPackageDto): number {
  return pkg.files.reduce((total, file) => total + file.size_bytes, 0)
}

function safeExternalUrl(value: string | undefined): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    return url.protocol === 'http:' || url.protocol === 'https:' ? value : null
  } catch {
    return null
  }
}

function hasPackageDetails(pkg: ModelCatalogPackageDto): boolean {
  return Boolean(safeExternalUrl(pkg.source_url) || pkg.license || !pkg.available)
}

function togglePackageDetails(packageId: string): void {
  if (expandedPackages.value.has(packageId)) expandedPackages.value.delete(packageId)
  else expandedPackages.value.add(packageId)
}

async function openModelPage(pkg: ModelCatalogPackageDto): Promise<void> {
  const url = safeExternalUrl(pkg.source_url)
  if (!url) return
  try {
    await openUrl(url)
  } catch {
    emit('show-message', t('settings.models.error.open_source'), 'error')
  }
}

function installedEntry(packageId: string) {
  return status.value.metadata.installed.find((item) => item.id === packageId)
}

function installedDate(pkg: ModelCatalogPackageDto): string | null {
  if (!isInstalled(pkg.id)) return null
  const entry = installedEntry(pkg.id)
  return entry ? formatDate(entry.installed_at) : null
}

function installedDateTime(pkg: ModelCatalogPackageDto): string {
  const entry = installedEntry(pkg.id)
  return entry ? formatDateTime(entry.installed_at) : ''
}

function installedVersionMismatch(pkg: ModelCatalogPackageDto): string | null {
  if (!isInstalled(pkg.id)) return null
  const entry = installedEntry(pkg.id)
  return entry && entry.version !== pkg.version ? entry.version : null
}

function isInstalled(packageId: string): boolean {
  return installedIds.value.has(packageId)
}

function isOutdated(packageId: string): boolean {
  return isInstalled(packageId) && outdatedIds.value.has(packageId)
}

function isActive(packageId: string): boolean {
  return Boolean(
    pendingInstall.value[packageId]
    || pendingCancel.value[packageId]
    || progressByPackage.value[packageId],
  )
}

function packageStatusText(pkg: ModelCatalogPackageDto): string {
  if (isOutdated(pkg.id)) return t('settings.models.status.outdated')
  if (isInstalled(pkg.id)) return t('settings.models.status.installed')
  if (isActive(pkg.id)) return t('settings.models.status.downloading')
  if (!pkg.available) return t('settings.models.status.unavailable')
  return t('settings.models.status.available')
}

function progressPercent(progress: ModelDownloadProgressDto): number | null {
  if (progress.total_bytes <= 0) return null
  return Math.min(100, Math.max(0, Math.floor((progress.received_bytes / progress.total_bytes) * 100)))
}

function progressText(progress: ModelDownloadProgressDto): string {
  if (progress.total_bytes <= 0) {
    return t('settings.models.progress.received', {
      received: formatBytes(progress.received_bytes),
    })
  }
  return t('settings.models.progress.details', {
    received: formatBytes(progress.received_bytes),
    total: formatBytes(progress.total_bytes),
  })
}

function progressValueText(progress: ModelDownloadProgressDto): string {
  const percent = progressPercent(progress)
  const details = progressText(progress)
  return percent === null ? details : `${percent}% · ${details}`
}

async function handleInstall(pkg: ModelCatalogPackageDto): Promise<void> {
  if (!pkg.available || installLocked.value) return
  const outcome = await installPackage(pkg.id)
  if (outcome === 'cancelled') {
    emit('show-message', t('settings.models.message.cancelled'), 'info')
    return
  }
  if (outcome === 'installed') {
    emit('show-message', t('settings.models.message.installed', { name: pkg.name }), 'success')
    if (pkg.category === 'ruaccent') {
      await accentorRef.value?.refreshFromDisk()
    }
    return
  }
  const error = errorByPackage.value[pkg.id]
  if (error) emit('show-message', error, 'error')
}

async function handleCancel(pkg: ModelCatalogPackageDto): Promise<void> {
  if (await cancelPackage(pkg.id)) return
  const error = errorByPackage.value[pkg.id]
  if (error) emit('show-message', error, 'error')
}

async function handleRefreshList(): Promise<void> {
  const refreshed = await refreshStatus()
  emit(
    'show-message',
    t(refreshed ? 'settings.models.message.list_refreshed' : 'settings.models.error.load'),
    refreshed ? 'success' : 'error',
  )
}

async function handleRefreshCatalog(): Promise<void> {
  const refreshed = await refreshCatalog()
  emit(
    'show-message',
    t(refreshed ? 'settings.models.message.catalog_refreshed' : 'settings.models.error.refresh_catalog'),
    refreshed ? 'success' : 'error',
  )
}

async function handleOpenFolder(): Promise<void> {
  const opened = await openFolder()
  if (!opened) emit('show-message', t('settings.models.error.open_folder'), 'error')
}

function forwardRuAccentMessage(message: string, severity?: MessageSeverity): void {
  emit('show-message', message, severity)
}
</script>

<template>
  <div class="settings-models">
    <section class="settings-section downloads-section">
      <div class="section-heading">
        <div class="section-heading-text">
          <h2 class="ui-section-title">{{ t('settings.models.downloads.title') }}</h2>
          <div v-if="catalogPublishedText" class="catalog-meta ui-metadata">
            <span :title="catalogFetchedTitle || undefined">
              {{ catalogPublishedText }}
            </span>
          </div>
        </div>
        <div class="section-actions">
          <button
            type="button"
            class="ui-icon-button"
            :disabled="statusLoading"
            :title="t('settings.models.action.refresh_list')"
            :aria-label="t('settings.models.action.refresh_list')"
            @click="handleRefreshList"
          >
            <RefreshCw :size="18" :class="{ 'ui-spin': statusLoading }" />
          </button>
          <button
            type="button"
            class="ui-button ui-button--compact ui-button--leading-icon"
            data-testid="download-catalog-button"
            :disabled="catalogRefreshPending || statusLoading"
            :title="t('settings.models.action.download_catalog')"
            @click="handleRefreshCatalog"
          >
            <Loader2 v-if="catalogRefreshPending" :size="16" class="ui-spin" />
            <Download v-else :size="16" />
            <span>{{ t('settings.models.action.download_catalog') }}</span>
          </button>
          <button
            type="button"
            class="ui-button ui-button--compact ui-button--leading-icon"
            :disabled="folderPending"
            :title="t('settings.models.downloads.manual_hint')"
            :aria-description="t('settings.models.downloads.manual_hint')"
            @click="handleOpenFolder"
          >
            <Loader2 v-if="folderPending" :size="16" class="ui-spin" />
            <FolderOpen v-else :size="16" />
            <span>{{ t('settings.models.action.open_folder') }}</span>
          </button>
        </div>
      </div>

      <div v-if="statusError" class="callout callout--danger" role="alert">
        <AlertTriangle :size="16" class="callout-icon" />
        <span class="callout-text">{{ t('settings.models.error.load') }}</span>
        <button
          type="button"
          class="ui-button ui-button--compact"
          :disabled="statusLoading"
          @click="handleRefreshList"
        >
          {{ t('settings.models.action.retry') }}
        </button>
      </div>
      <div v-else-if="statusLoading && !statusLoaded" class="ui-status loading-line" role="status">
        <Loader2 :size="16" class="ui-spin" />
        <span>{{ t('settings.models.loading') }}</span>
      </div>

      <div
        v-if="statusLoaded && !statusError && !status.catalog"
        class="callout callout--info empty-catalog"
        data-testid="empty-catalog-callout"
        role="status"
      >
        <Download :size="16" class="callout-icon" aria-hidden="true" />
        <span class="callout-text">{{ t('settings.models.catalog.empty') }}</span>
        <button
          type="button"
          class="ui-button ui-button--compact ui-button--leading-icon"
          :disabled="catalogRefreshPending || statusLoading"
          @click="handleRefreshCatalog"
        >
          <Loader2 v-if="catalogRefreshPending" :size="16" class="ui-spin" />
          <Download v-else :size="16" />
          <span>{{ t('settings.models.action.download_catalog') }}</span>
        </button>
      </div>

      <template v-if="status.catalog">
      <div
        v-for="group in groupDefinitions"
        :key="group.category"
        :ref="(element) => setGroupElement(group.category, element)"
        class="models-group"
        :aria-labelledby="`models-group-${group.category}`"
        tabindex="-1"
      >
        <h3 :id="`models-group-${group.category}`" class="ui-group-title group-title">
          {{ t(group.labelKey) }}
        </h3>
        <div v-if="packagesByCategory[group.category].length > 0" class="group-content">
          <template
            v-for="section in categorySections(group.category)"
            :key="section.key"
          >
            <button
              v-if="section.key === 'extras'"
              type="button"
              class="extras-toggle"
              :aria-expanded="expandedExtras.has(group.category)"
              :aria-controls="`models-extras-${group.category}`"
              @click="toggleGroupExtras(group.category)"
            >
              <ChevronDown :size="14" class="extras-chevron" aria-hidden="true" />
              <span>{{ t('settings.models.group.more', { count: section.items.length }) }}</span>
            </button>
            <div
              v-if="section.key === 'primary' || expandedExtras.has(group.category)"
              :id="section.key === 'extras' ? `models-extras-${group.category}` : undefined"
              class="package-list"
              :class="{ 'package-list--extras': section.key === 'extras' }"
            >
              <article
                v-for="pkg in section.items"
                :key="pkg.id"
                class="package-row"
                :data-package-id="pkg.id"
              >
                <div class="package-main">
                  <div class="package-summary">
                    <div class="package-title-row">
                      <h4 class="package-name">{{ pkg.name }}</h4>
                      <span v-if="pkg.recommended" class="package-recommended">
                        {{ t('settings.models.package.recommended') }}
                      </span>
                    </div>
                    <div class="package-meta ui-metadata">
                  <span>v{{ pkg.version }}</span>
                  <template v-if="pkg.files.length > 0">
                    <span class="meta-sep" aria-hidden="true">·</span>
                    <span>{{ formatBytes(packageSize(pkg)) }}</span>
                  </template>
                  <span class="meta-sep" aria-hidden="true">·</span>
                  <span
                    class="package-status"
                    :class="{
                      'package-status--outdated': isOutdated(pkg.id),
                      'package-status--installed': isInstalled(pkg.id) && !isOutdated(pkg.id),
                    }"
                    role="status"
                  >
                    <AlertTriangle v-if="isOutdated(pkg.id)" :size="14" aria-hidden="true" />
                    <Check v-else-if="isInstalled(pkg.id)" :size="14" aria-hidden="true" />
                    {{ packageStatusText(pkg) }}
                  </span>
                  <template v-if="installedVersionMismatch(pkg)">
                    <span class="meta-sep" aria-hidden="true">·</span>
                    <span>
                      {{ t('settings.models.status.installed_version', { version: installedVersionMismatch(pkg) }) }}
                    </span>
                  </template>
                  <template v-if="installedDate(pkg)">
                    <span class="meta-sep" aria-hidden="true">·</span>
                    <span :title="installedDateTime(pkg)">
                      {{ t('settings.models.status.installed_at', { date: installedDate(pkg) }) }}
                    </span>
                  </template>
                </div>
              </div>

              <div class="package-controls">
                <template v-if="pkg.available || isActive(pkg.id)">
                  <div class="package-actions">
                    <template v-if="isActive(pkg.id)">
                      <button
                        type="button"
                        class="ui-button ui-button--compact ui-button--leading-icon ui-action--stop"
                        :disabled="pendingCancel[pkg.id]"
                        @click="handleCancel(pkg)"
                      >
                        <Loader2 v-if="pendingCancel[pkg.id]" :size="16" class="ui-spin" />
                        <X v-else :size="16" />
                        <span>{{ t('settings.models.action.cancel') }}</span>
                      </button>
                    </template>
                    <template v-else-if="errorByPackage[pkg.id]">
                      <button
                        type="button"
                        class="ui-button ui-button--compact ui-button--leading-icon"
                        :disabled="installLocked"
                        @click="handleInstall(pkg)"
                      >
                        <RotateCcw :size="16" />
                        <span>{{ t('settings.models.action.retry') }}</span>
                      </button>
                    </template>
                    <template v-else>
                      <button
                        type="button"
                        class="ui-button ui-button--compact ui-button--leading-icon"
                        :disabled="installLocked"
                        @click="handleInstall(pkg)"
                      >
                        <Download :size="16" />
                        <span>{{ t(isInstalled(pkg.id) ? 'settings.models.action.reinstall' : 'settings.models.action.download') }}</span>
                      </button>
                    </template>
                  </div>
                  <button
                    v-if="hasPackageDetails(pkg)"
                    type="button"
                    class="package-details-toggle"
                    :aria-expanded="expandedPackages.has(pkg.id)"
                    :aria-controls="`model-details-${pkg.id}`"
                    @click="togglePackageDetails(pkg.id)"
                  >
                    <span>{{ t('settings.models.package.details') }}</span>
                    <ChevronDown :size="14" class="package-details-chevron" aria-hidden="true" />
                  </button>
                </template>
                <button
                  v-else-if="hasPackageDetails(pkg)"
                  type="button"
                  class="ui-button ui-button--compact package-details-button"
                  :aria-expanded="expandedPackages.has(pkg.id)"
                  :aria-controls="`model-details-${pkg.id}`"
                  @click="togglePackageDetails(pkg.id)"
                >
                  <span>{{ t('settings.models.package.details') }}</span>
                  <ChevronDown :size="14" class="package-details-chevron" aria-hidden="true" />
                </button>
              </div>
            </div>

            <div
              v-show="expandedPackages.has(pkg.id)"
              :id="`model-details-${pkg.id}`"
              class="package-details ui-hint"
            >
              <div v-if="safeExternalUrl(pkg.source_url) || pkg.license" class="package-details-meta">
                <a
                  v-if="safeExternalUrl(pkg.source_url)"
                  class="package-source"
                  :href="safeExternalUrl(pkg.source_url) ?? undefined"
                  :title="safeExternalUrl(pkg.source_url) ?? undefined"
                  target="_blank"
                  rel="noopener noreferrer"
                  @click.prevent="openModelPage(pkg)"
                >
                  <span>{{ t('settings.models.package.source') }}</span>
                  <ExternalLink :size="13" aria-hidden="true" />
                </a>
                <span v-if="pkg.license">{{ t('settings.models.package.license', { license: pkg.license }) }}</span>
              </div>
              <p v-if="!pkg.available" class="package-unavailable">
                <strong>{{ t('settings.models.package.download_unavailable') }}</strong>
                <span v-if="pkg.unavailable_reason">{{ pkg.unavailable_reason }}</span>
              </p>
            </div>

            <p v-if="errorByPackage[pkg.id]" class="package-error" role="alert">
              <AlertTriangle :size="14" />
              <span>{{ errorByPackage[pkg.id] }}</span>
            </p>
            <p
              v-else-if="isOutdated(pkg.id) && !isActive(pkg.id)"
              class="package-warning"
              role="status"
            >
              <AlertTriangle :size="14" aria-hidden="true" />
              <span>{{ t('settings.models.package.outdated_hint') }}</span>
            </p>

            <div v-if="isActive(pkg.id)" class="download-progress">
              <div
                class="progress-track"
                role="progressbar"
                :aria-label="`${pkg.name}: ${packageStatusText(pkg)}`"
                aria-valuemin="0"
                :aria-valuemax="progressByPackage[pkg.id]?.total_bytes || undefined"
                :aria-valuenow="progressByPackage[pkg.id]?.received_bytes"
                :aria-valuetext="progressByPackage[pkg.id] ? progressValueText(progressByPackage[pkg.id]) : t('settings.models.progress.preparing')"
              >
                <div
                  v-if="progressByPackage[pkg.id]"
                  class="progress-fill"
                  :style="{ width: `${progressPercent(progressByPackage[pkg.id]) ?? 0}%` }"
                />
              </div>
              <span class="progress-text ui-metadata" role="status" aria-live="polite">
                <template v-if="pendingCancel[pkg.id]">
                  {{ t('settings.models.message.cancelling') }}
                </template>
                <template v-else-if="progressByPackage[pkg.id]">
                  {{ progressValueText(progressByPackage[pkg.id]) }}
                </template>
                <template v-else>
                  {{ t('settings.models.progress.preparing') }}
                </template>
              </span>
            </div>
          </article>
            </div>
          </template>
        </div>
        <p
          v-else-if="statusLoaded && !statusLoading"
          class="ui-hint group-empty"
          role="status"
        >
          {{ t('settings.models.group.empty') }}
        </p>
      </div>
      </template>
    </section>

    <SettingsRuAccent ref="accentorRef" @show-message="forwardRuAccentMessage" />
  </div>
</template>

<style scoped>
.settings-models {
  display: flex;
  flex-direction: column;
  gap: var(--ui-section-gap);
}

.settings-section {
  padding: var(--ui-section-padding-y) var(--ui-section-padding-x);
  background: var(--color-bg-field);
  border: 1px solid var(--color-border);
  border-radius: 12px;
  backdrop-filter: blur(8px);
}

.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-row-label-gap-side);
  flex-wrap: wrap;
}

.section-heading-text {
  min-width: 0;
  flex: 1 1 10rem;
}

.section-heading h2 {
  margin: 0;
}

.section-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-field-group-gap);
}

.catalog-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin-top: 2px;
}

.callout {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-field-group-gap);
  padding: 0.6rem 0.75rem;
  margin-top: var(--ui-section-gap);
  border-radius: 8px;
  font-size: var(--ui-text-size-hint);
  line-height: 1.4;
}

.callout--danger {
  background: var(--danger-bg-weak);
  border: 1px solid var(--danger-border);
  color: var(--danger-text-bright);
}

.callout--info {
  background: var(--info-bg-weak);
  border: 1px solid var(--info-border);
  color: var(--color-text-primary);
}

.callout-icon {
  flex-shrink: 0;
}

.callout-text {
  flex: 1 1 220px;
  min-width: 0;
  overflow-wrap: anywhere;
}

.loading-line {
  display: flex;
  align-items: center;
  gap: var(--ui-field-group-gap);
  margin-top: var(--ui-section-gap);
}

.models-group {
  padding-top: var(--ui-section-gap);
  margin-top: var(--ui-section-gap);
  border-top: 1px solid var(--color-border);
  scroll-margin-top: var(--ui-section-gap);
}

.models-group:focus {
  outline: 2px solid var(--color-accent);
  outline-offset: 4px;
  border-radius: 4px;
}

.group-title {
  margin: 0 0 var(--ui-hint-gap);
}

.package-row {
  display: flex;
  flex-direction: column;
  gap: var(--ui-hint-gap);
  padding: 8px 0;
}

.package-row + .package-row {
  border-top: 1px solid var(--color-border);
}

.package-main {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 6px 12px;
}

.package-summary {
  flex: 1 1 13rem;
  min-width: 0;
}

.package-title-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 8px;
}

.package-recommended {
  display: inline-flex;
  align-items: center;
  padding: 1px 8px;
  border-radius: 999px;
  border: 1px solid var(--info-border);
  background: var(--info-bg-weak);
  color: var(--color-accent);
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-label);
  line-height: 1.3;
}

.package-name {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
  font-size: var(--ui-text-size-label);
  font-weight: var(--ui-text-weight-group-title);
  line-height: var(--ui-line-height-multiline);
  color: var(--color-text-primary);
}

.group-content {
  display: flex;
  flex-direction: column;
}

.extras-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 7px 10px;
  margin-top: 4px;
  border: 1px solid var(--color-border);
  border-radius: var(--ui-radius);
  background: var(--color-bg-field);
  color: var(--color-text-secondary);
  font: inherit;
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-label);
  cursor: pointer;
  text-align: left;
  transition: background-color 150ms ease, color 150ms ease;
}

.extras-toggle:hover {
  background: var(--color-bg-field-hover);
  color: var(--color-text-primary);
}

.extras-toggle:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.extras-chevron {
  flex-shrink: 0;
  transition: transform 150ms ease;
}

.extras-toggle[aria-expanded="true"] .extras-chevron {
  transform: rotate(180deg);
}

.package-list--extras {
  margin-top: 2px;
  padding-left: 10px;
  border-left: 2px solid var(--color-border);
}

.package-meta {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 2px 6px;
}

.meta-sep {
  opacity: 0.65;
  user-select: none;
}

.package-controls {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  margin-left: auto;
  flex-shrink: 0;
}

.package-details-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 24px;
  padding: 2px 4px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--color-text-secondary);
  font: inherit;
  font-size: var(--ui-text-size-hint);
  cursor: pointer;
}

.package-details-toggle:hover {
  color: var(--color-text-primary);
}

.package-details-toggle:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}

.package-details-button {
  gap: 6px;
}

.package-details-button svg {
  width: 14px;
  height: 14px;
}

.package-details-chevron {
  transition: transform 150ms ease;
}

.package-details-toggle[aria-expanded="true"] .package-details-chevron,
.package-details-button[aria-expanded="true"] .package-details-chevron {
  transform: rotate(180deg);
}

.package-details {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 0;
  padding: 4px 0 2px 10px;
  border-left: 2px solid var(--color-border);
  overflow-wrap: anywhere;
}

.package-details-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 12px;
}

.package-source {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--color-accent);
  text-decoration: underline;
  text-underline-offset: 2px;
}

.package-source:hover {
  opacity: 0.85;
}

.package-source:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
  border-radius: 4px;
}

.package-unavailable {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0;
}

.package-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.package-status--installed {
  color: var(--success-text-bright);
}

.package-status--outdated {
  color: var(--warning-text-bright);
}

.package-warning {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  margin: 0;
  color: var(--warning-text-bright);
  font-size: var(--ui-text-size-hint);
  line-height: var(--ui-line-height-multiline);
  overflow-wrap: anywhere;
}

.package-warning svg {
  flex-shrink: 0;
  margin-top: 2px;
}

.package-error {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  margin: 0;
  color: var(--danger-text-bright);
  font-size: var(--ui-text-size-hint);
  line-height: var(--ui-line-height-multiline);
  overflow-wrap: anywhere;
}

.package-error svg {
  flex-shrink: 0;
  margin-top: 2px;
}

.download-progress {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.progress-track {
  height: 6px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--btn-neutral-bg);
}

.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--color-accent);
  transition: width 150ms ease;
}

@media (prefers-reduced-motion: reduce) {
  .progress-fill,
  .package-details-chevron,
  .extras-toggle,
  .extras-chevron {
    transition: none;
  }
}

.progress-text {
  overflow-wrap: anywhere;
}

.group-empty {
  margin: 0;
}

.empty-catalog {
  margin: var(--ui-section-gap) 0 0;
}
</style>
