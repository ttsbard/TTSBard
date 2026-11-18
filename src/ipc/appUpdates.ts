/**
 * IPC contract for update checks through GitHub Releases (ROADMAP-141).
 *
 * The backend owns the SemVer comparison and the schedule; every command
 * returns the full `UpdateStatusDto` snapshot, so the card and the banner are
 * always driven by one state object. Check failures arrive as stable codes
 * inside the DTO (never as command rejections), and the card maps any of them
 * to one fixed, calm locale phrase so raw backend text never reaches the
 * screen.
 */

/** Wire event carrying `UpdateStatusDto`; matches `updates.rs` on the backend. */
export const UPDATE_STATUS_CHANGED_EVENT = 'update-status-changed'

export const GET_UPDATE_STATUS_COMMAND = 'get_update_status'
export const CHECK_FOR_UPDATES_COMMAND = 'check_for_updates'
export const SET_UPDATES_STARTUP_CHECK_COMMAND = 'set_updates_startup_check'
export const SET_UPDATES_PERIODIC_CHECK_COMMAND = 'set_updates_periodic_check'
export const SET_UPDATES_CHECK_INTERVAL_COMMAND = 'set_updates_check_interval'
export const DISMISS_UPDATE_PANEL_COMMAND = 'dismiss_update_panel'

/** In-app update installation (ROADMAP-141, stage 2). */
export const INSTALL_UPDATE_COMMAND = 'install_update'
export const CANCEL_UPDATE_INSTALL_COMMAND = 'cancel_update_install'
/** Wire event carrying `UpdateInstallProgressDto`; matches `install.rs`. */
export const UPDATE_INSTALL_PROGRESS_EVENT = 'update-install-progress'

/** Automatic check interval bounds, mirrored by the backend clamp (1..=7). */
export const UPDATES_MIN_INTERVAL_DAYS = 1
export const UPDATES_MAX_INTERVAL_DAYS = 7

/** A stable GitHub release offered to the user. */
export interface ReleaseInfoDto {
  /** Release version without the leading `v`, parseable as SemVer. */
  version: string
  /** Release title, if the maintainer set one. */
  name: string | null
  /** Release notes as plain text; rendered read-only, never as HTML. */
  notes: string
  /** Release page on github.com. */
  url: string
  /** RFC3339 publish time as reported by GitHub. */
  published_at: string | null
}

/**
 * Stable code of the last failed check. Keep in sync with
 * `UpdateErrorKind::as_str` in `src-tauri/src/updates/mod.rs`.
 */
export type UpdateErrorCode = 'network' | 'rate_limited' | 'not_found' | 'parse' | 'save'

/** Full update state: persisted schedule plus the last completed attempt. */
export interface UpdateStatusDto {
  current_version: string
  /** The known latest release is newer than the running version. */
  update_available: boolean
  latest: ReleaseInfoDto | null
  /**
   * RFC3339 (UTC) of the last completed attempt (success or failure, manual
   * included); `null` — no attempt has completed yet.
   */
  last_attempt_at: string | null
  /** Stable code of the last failed attempt; `null` — it succeeded or no attempt has completed yet. */
  last_attempt_error: UpdateErrorCode | null
  /** RFC3339 (UTC) of the last successful check, including manual ones. Unchanged by a failed attempt. */
  last_check_at: string | null
  checking: boolean
  /** Persisted one-shot check-at-process-startup switch. */
  startup_check_enabled: boolean
  /** Persisted periodic check switch; the selected interval is kept when off. */
  periodic_check_enabled: boolean
  check_interval_days: number
  /** Version whose update panel the user closed. */
  dismissed_version: string | null
  /**
   * In-app installation is offered for the found release: updater public key
   * configured, the running app lives in the recorded NSIS install directory
   * and the updater artifact is confirmed available (see
   * `updates::install::InstallState`).
   */
  install_available: boolean
}

/**
 * Stable code of a failed install. Keep in sync with
 * `InstallErrorKind::as_str` in `src-tauri/src/updates/install.rs`.
 */
export type UpdateInstallErrorCode =
  | 'unavailable'
  | 'network'
  | 'not_found'
  | 'signature'
  | 'filesystem'
  | 'launch'
  | 'no_update'

/** Pipeline stage names of the install progress event. */
export type UpdateInstallStage =
  | 'checking'
  | 'downloading'
  | 'saving'
  | 'launching'
  | 'cancelled'
  | 'failed'

/** Progress payload pushed by the backend install pipeline. */
export interface UpdateInstallProgressDto {
  stage: UpdateInstallStage
  version: string | null
  downloaded: number
  total: number | null
  error_code: UpdateInstallErrorCode | null
}

/** Locale key per stable install error code, plus the generic fallback. */
export function updateInstallErrorLocaleKey(code: string | null | undefined): string {
  switch (code) {
    case 'unavailable':
      return 'general.updates.install.errors.unavailable'
    case 'network':
      return 'general.updates.install.errors.network'
    case 'not_found':
      return 'general.updates.install.errors.not_found'
    case 'signature':
      return 'general.updates.install.errors.signature'
    case 'filesystem':
      return 'general.updates.install.errors.filesystem'
    case 'launch':
      return 'general.updates.install.errors.launch'
    case 'no_update':
      return 'general.updates.install.errors.no_update'
    default:
      return 'general.updates.install.errors.generic'
  }
}

/** Locale key per pipeline stage. */
export function updateInstallStageLocaleKey(stage: UpdateInstallStage): string {
  return `general.updates.install.stage.${stage}`
}

/**
 * Locale key of the calm in-card plate shown for every failed check. The
 * stable code stays in the DTO and the backend log; the card deliberately
 * shows one fixed phrase instead of per-code variants, while settings-save
 * and install errors keep their own texts.
 */
export const UPDATE_CHECK_ERROR_LOCALE_KEY = 'general.updates.errors.check_failed'

function isReleaseInfo(value: unknown): value is ReleaseInfoDto {
  if (typeof value !== 'object' || value === null) return false
  const release = value as Record<string, unknown>
  return typeof release.version === 'string'
    && typeof release.notes === 'string'
    && typeof release.url === 'string'
    && (release.name === null || typeof release.name === 'string')
    && (release.published_at === null || typeof release.published_at === 'string')
}

function isUpdateErrorCode(value: unknown): value is UpdateErrorCode {
  return value === 'network' || value === 'rate_limited'
    || value === 'not_found' || value === 'parse' || value === 'save'
}

const UPDATE_INSTALL_STAGES: readonly UpdateInstallStage[] = [
  'checking',
  'downloading',
  'saving',
  'launching',
  'cancelled',
  'failed',
]

const UPDATE_INSTALL_ERROR_CODES: readonly UpdateInstallErrorCode[] = [
  'unavailable',
  'network',
  'not_found',
  'signature',
  'filesystem',
  'launch',
  'no_update',
]

/**
 * Runtime guard for the install progress payload: malformed events are
 * dropped so the modal never renders unknown stages.
 */
export function parseUpdateInstallProgress(value: unknown): UpdateInstallProgressDto | null {
  if (typeof value !== 'object' || value === null) return null
  const progress = value as Record<string, unknown>
  const stage = progress.stage
  const errorCode = progress.error_code
  if (typeof stage !== 'string' || !UPDATE_INSTALL_STAGES.includes(stage as UpdateInstallStage)) {
    return null
  }
  if (errorCode !== null && !UPDATE_INSTALL_ERROR_CODES.includes(errorCode as UpdateInstallErrorCode)) {
    return null
  }
  return {
    stage: stage as UpdateInstallStage,
    version: typeof progress.version === 'string' ? progress.version : null,
    downloaded: typeof progress.downloaded === 'number' ? progress.downloaded : 0,
    total: typeof progress.total === 'number' ? progress.total : null,
    error_code: (errorCode ?? null) as UpdateInstallErrorCode | null,
  }
}

/**
 * Runtime guard for the wire payload: malformed snapshots are dropped instead
 * of partially applied, so the card and the banner never render unknown data.
 */
export function parseUpdateStatus(value: unknown): UpdateStatusDto | null {
  if (typeof value !== 'object' || value === null) return null
  const status = value as Record<string, unknown>
  if (typeof status.current_version !== 'string'
    || typeof status.update_available !== 'boolean'
    || typeof status.checking !== 'boolean'
    || typeof status.startup_check_enabled !== 'boolean'
    || typeof status.periodic_check_enabled !== 'boolean'
    || typeof status.check_interval_days !== 'number'
    || typeof status.install_available !== 'boolean'
    || (status.latest !== null && !isReleaseInfo(status.latest))
    || (status.last_attempt_at !== null && typeof status.last_attempt_at !== 'string')
    || (status.last_attempt_error !== null && !isUpdateErrorCode(status.last_attempt_error))
    || (status.last_check_at !== null && typeof status.last_check_at !== 'string')
    || (status.dismissed_version !== null && typeof status.dismissed_version !== 'string')) {
    return null
  }
  return status as unknown as UpdateStatusDto
}
