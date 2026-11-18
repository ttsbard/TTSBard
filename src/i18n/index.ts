import { ref, type Ref } from 'vue'
import { createI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import englishCatalog from '../../locales/en.json'

type Language = { locale: string; name: string }

type LocalizationSnapshot = {
  revision: number
  requestedLocale: string
  locale: string
  languages: Language[]
  messages: Record<string, string>
  diagnostics: string[]
}

const englishMessages = englishCatalog.messages as Record<string, string>

export const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: 'en',
  fallbackLocale: 'en',
  flatJson: true,
  messages: { en: englishMessages },
  // Slavic plural rule for Russian («день | дня | дней»); other locales keep
  // the default rule (documented vue-i18n pluralization recipe).
  pluralRules: {
    ru: (choice: number, choicesLength: number): number => {
      const n = Math.floor(Math.abs(choice))
      if (n % 10 === 1 && n % 100 !== 11) return 0
      if ([2, 3, 4].includes(n % 10) && ![12, 13, 14].includes(n % 100)) {
        return Math.min(1, choicesLength - 1)
      }
      return choicesLength - 1
    },
  },
})

export const locale = ref('en')
export const requestedLocale = ref('en')
const builtinLanguages: Language[] = [
  { locale: 'en', name: 'English' },
  { locale: 'ru', name: 'Русский' },
]

export const availableLanguages = ref<Language[]>(builtinLanguages)
export const bootstrapError = ref<string | null>(null)

let latestRevision = -1

function isStringRecord(value: unknown): value is Record<string, string> {
  return typeof value === 'object' && value !== null
    && Object.values(value).every((entry) => typeof entry === 'string')
}

function isSnapshot(value: unknown): value is LocalizationSnapshot {
  if (typeof value !== 'object' || value === null) return false
  const snapshot = value as Record<string, unknown>
  return typeof snapshot.revision === 'number'
    && typeof snapshot.requestedLocale === 'string'
    && typeof snapshot.locale === 'string'
    && Array.isArray(snapshot.languages)
    && snapshot.languages.every((language) => typeof language === 'object'
      && language !== null
      && typeof (language as Record<string, unknown>).locale === 'string'
      && typeof (language as Record<string, unknown>).name === 'string')
    && isStringRecord(snapshot.messages)
    && Array.isArray(snapshot.diagnostics)
    && snapshot.diagnostics.every((diagnostic) => typeof diagnostic === 'string')
}

function applySnapshot(snapshot: LocalizationSnapshot): boolean {
  if (snapshot.revision < latestRevision) return false

  latestRevision = snapshot.revision
  i18n.global.setLocaleMessage(snapshot.locale, snapshot.messages)
  ;(i18n.global.locale as Ref<string>).value = snapshot.locale
  locale.value = snapshot.locale
  requestedLocale.value = snapshot.requestedLocale
  const languagesByLocale = new Map(builtinLanguages.map((language) => [language.locale, language]))
  for (const language of snapshot.languages) languagesByLocale.set(language.locale, language)
  availableLanguages.value = [...languagesByLocale.values()]
    .sort((left, right) => left.locale.localeCompare(right.locale))

  if (typeof document !== 'undefined') {
    document.documentElement.lang = snapshot.locale
  }
  return true
}

export function t(key: string, values?: Record<string, unknown> | number): string {
  if (values === undefined) return i18n.global.t(key)
  return typeof values === 'number' ? i18n.global.t(key, values) : i18n.global.t(key, values)
}

export function disposeLocalization(): void {
  // Localization is immutable for the lifetime of a running application.
}

export async function bootstrapLocalization(): Promise<void> {
  disposeLocalization()
  bootstrapError.value = null

  try {
    // A hidden Tauri window can finish loading a few milliseconds before the
    // setup callback has registered LocalizationState. Do not permanently
    // fall back to English for that startup race: wait briefly for the state
    // that was already built from persisted settings.
    let snapshot: unknown
    let lastError: unknown
    for (let attempt = 0; attempt < 20; attempt++) {
      try {
        snapshot = await invoke<unknown>('get_localization')
        break
      } catch (error) {
        lastError = error
        await new Promise<void>((resolve) => window.setTimeout(resolve, 25))
      }
    }
    if (snapshot === undefined) throw lastError ?? new Error('Localization service is unavailable')
    if (!isSnapshot(snapshot)) throw new Error('Localization service returned an invalid response')
    applySnapshot(snapshot)
  } catch (error) {
    bootstrapError.value = error instanceof Error ? error.message : String(error)
  }
}

export async function setLanguage(code: string): Promise<void> {
  await invoke<void>('set_ui_language', { locale: code })
}
