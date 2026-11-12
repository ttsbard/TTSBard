# Карта UI

Навигация по интерфейсу для исследования и постановки локальных задач.
Карта описывает исходники, а не подтверждает прохождение runtime-проверок.
Правила проверки и статус визуальных ориентиров — в [UI guidelines](./ui-guidelines.md).
Владельцы CSS, исключения и группы излишней кастомизации — в
[реестре CSS](./ui-css-ownership.md).

## Основное окно

Вход: `src/main.ts` → `src/App.vue`. Навигация: `src/components/Sidebar.vue`.
Идентификаторы панелей определены типом `Panel` в этих компонентах.
Порядок групп Sidebar: ввод/синтез/аудио → интеграции → вспомогательные
инструменты: «Быстрая вставка» (`preprocessor`), горячие клавиши, перехват,
настройки. «Быстрая вставка» не отделяется от нижней группы разделителем.
Пути в таблице относительно `src/components/`.

| ID | Экран | Компонент | Вложенные поверхности |
|---|---|---|---|
| `input` | Ввод | `InputPanel.vue` | `editor/TtsEditor.vue`, `EditorTabs.vue`, `EditorMenu.vue`, `RouteSelector.vue`, `IncomingTextsTab.vue`, `IncomingRouteSelector.vue`, `SpellContextMenu.vue`, `PhraseHistoryList.vue` |
| `tts` | Синтез речи | `TtsPanel.vue` | `tts/TtsOpenAICard.vue`, `TtsSileroCard.vue`, `TtsLocalCard.vue`, `TtsPocketCard.vue`, `TtsFishAudioCard.vue`, `TtsElevenLabsCard.vue`, `VoiceSelector.vue`, `FishAudioModelPicker.vue`, `TelegramConnectionStatus.vue`, `TelegramAuthModal.vue` |
| `audio` | Аудио | `AudioPanel.vue` | `audio/AudioDevicesTab.vue`, `AudioEffectsTab.vue`, `AudioPreviewBar.vue`, `DspSettings.vue`, `EqSettings.vue`, `CompressorSettings.vue`, `LimiterSettings.vue`, `EffectsSettings.vue` |
| `preprocessor` | Препроцессор | `PreprocessorPanel.vue` | Правила обработки и их условные настройки |
| `webview` | WebView | `WebViewPanel.vue` | Сервер с постоянным LAN bind, порт, локальный/LAN URL; шаблоны, тест, внешнее подключение и UPnP |
| `twitch` | Twitch | `TwitchPanel.vue` | Режимы подключения (IRC и Twitch API), реквизиты в блоке подключения; отдельный блок «Аккаунты» для режима API: авторизация бота через Device Code Flow, локальный сброс бота с сохранением реквизитов и каналов, список авторизованных каналов с выбором одного получателя и автоматическим обновлением активного API-подключения при смене канала и локальным удалением доступа, добавление канала через Device Code Flow с копированием/открытием ссылки активации, ошибки авторизации и хранилища, тест |
| `vtube-studio` | VTube Studio | `VTubeStudioPanel.vue` | Адрес/порт, параметры и действия при наборе, тестирование |
| `vrchat` | VRChat | `VRChatPanel.vue` | Адрес и порт OSC, запуск при старте, тестовое сообщение, помощь |
| `ocr` | OCR | `OcrPanel.vue` | Настройки распознавания, выбор экрана захвата и выбор области |
| `input-server` | Входящий сервер | `InputServerPanel.vue` | Подключение и маршрутизация входящего текста |
| `hotkeys` | Горячие клавиши | `HotkeysPanel.vue` | Назначение сочетаний |
| `intercept` | Перехват | `InterceptPanel.vue` | Настройки перехвата |
| `settings` | Настройки | `SettingsPanel.vue` | Вкладки ниже |

## Настройки

| ID вкладки | Компонент | Связанные поверхности |
|---|---|---|
| `general` | `src/components/settings/SettingsGeneral.vue` | Язык, поведение окон, кнопки дополнительных окон (скрытие в заголовке), папки, диагностика; `DataTransferModal.vue` в том же каталоге |
| `interface` | `src/components/settings/SettingsInterface.vue` | Тема и внешний вид окон |
| `editor` | `src/components/settings/SettingsEditor.vue` | `EditorFontSettings.vue` в том же каталоге; карточка преобразования чисел сразу после шрифтов |
| `network` | `src/components/settings/SettingsNetwork.vue` | Сетевые формы |
| `ai` | `src/components/SettingsAiPanel.vue` | Настройки AI |

## Общие поверхности и отдельные окна

| Поверхность | Источник |
|---|---|
| Titlebar и переключение дополнительных окон | `src/App.vue` |
| Статусы интеграций | `src/components/titlebar/IntegrationStatusCluster.vue`, проекция WebView, Twitch, VTube Studio, VRChat (включённый исходящий OSC) и входящего сервера — `src/composables/useIntegrationStatusSlots.ts` |
| Уведомления | `src/components/ErrorToasts.vue`, `shared/StatusMessage.vue`, `shared/TestResult.vue` в `src/components/` |
| Компактный режим | `src/composables/mainWindowMode.ts` (единый владелец), `src/composables/mainWindowModeAdapter.ts` (native IPC), `src/components/MinimalModeButton.vue` (кнопка), `src/composables/useCompactWindowResize.ts`, `src/components/InputPanel.vue` — изменение размера за края окна; внутренние захваты в компактном режиме отсутствуют, разделитель высоты редактора доступен в обычном режиме. Меню `EditorMenu.vue` позволяет перейти в моно из компактного и обычного окна, а также независимо скрыть/показать панель вкладок; предпочтение видимости хранится в localStorage (`app-editor-tabs-visible`), сохраняется при выходе из моно. На поверхности входящих вкладки остаются доступны для возврата к тексту. При скрытых вкладках редактор имеет скругление всех углов 6 px. Список маршрутов в нижней панели раскрывается вверх |
| Режим «Моно» | `src/components/MonoModeBar.vue`, `src/composables/mainWindowMode.ts`, `src/composables/useIntegrationStatusSlots.ts`, `src/App.vue`, `src/components/InputPanel.vue` — стиль компактного режима: скрыты titlebar, вкладки, action bar и история. Угловая `MinimalModeButton` остаётся для прямого возврата в обычное окно. Сверху отображается `MonoModeBar` (18 px) с центральной ручкой, кнопкой `ChevronDown` для возврата в компактный вид, кнопкой сворачивания окна и значками только активных ошибок интеграций с popover подробностей без смещения редактора или расширения окна. При переходе между компактным видом и моно геометрия окна сохраняется без ресайза; переход из обычного окна применяет сохранённый компактный размер |
| Звуковая панель | `src-soundpanel/main.ts` → `SoundPanelApp.vue` в том же каталоге |
| Управление воспроизведением | `src-playback/main.ts` → `PlaybackControlApp.vue` в том же каталоге |
| Выбор области OCR | `src-ocr-selection/main.ts` → `SelectionApp.vue` в том же каталоге |
| Браузерный overlay | `src-tauri/src/input_server/overlay.html` |

## Где искать оформление и поведение

- Главное окно: `src/style.css` и `src/styles/index.css`; цвета и переменные — `src/styles/variables.css`; базовые стили — `base.css`; общий фокус — `focus.css` в `src/styles/`.
- Согласованные размеры и отступы основных форм — `src/styles/ui-tokens.css`; общий слой явно подключаемых классов `ui-*` — `src/styles/ui-controls.css`. Переведены на слой: вкладки настроек и аудио, все вкладки настроек, TTS-карточки и диалоги, OCR, препроцессор, перехват и горячие клавиши (обычные роли), аудио-устройства/эффекты (обычные роли), интеграции, шрифтовые пикеры. Компактные поверхности редактора/истории, титлбар и отдельные окна сохраняют собственную плотность (предложение шкалы — в ROADMAP-121); глобальный `<style>` в `AudioPanel.vue` остаётся владельцем стилей плотных DSP-подсекций.
- Общие контролы: `src/components/shared/InputWithToggle.vue`, `ProviderCard.vue`, `StatusMessage.vue`, `TestResult.vue` в том же каталоге.
- Редактор текста: `editor/TtsEditor.vue` — нижний радиус 6 px; в обычном режиме фокус обозначается приглушённой границей без свечения. В компактном режиме и «Моно» `InputPanel.vue` полностью убирает оформление фокуса редактора, включая клавиатурный фокус; каретка и выделение сохраняются. В «Моно» поле постоянно обозначено нейтральной рамкой 1 px (`rgba(var(--rgb-border), 0.22)`) по всем сторонам, одинаковой при фокусе и без него.
- Аудио: обновление списка устройств находится справа от вкладок в `AudioPanel.vue`, вне `tablist`, и показывается только на вкладке «Устройства». Загрузка и обновление списка остаются в `audio/AudioDevicesTab.vue`; во время обновления кнопка заблокирована и показывает spinner. Вкладки могут переноситься внутри оставшейся ширины строки.
- Длинные URL: ссылки активации OAuth бота/канала в `TwitchPanel.vue` и локальный/LAN/внешний адреса в `WebViewPanel.vue` используют однострочный `input type="text" readonly` внутри `ui-composite`: нативное выделение, Ctrl+A и горизонтальная прокрутка при перемещении курсора, полный отображаемый адрес в `title`. Правило и статус проверки — в [UI guidelines](./ui-guidelines.md).
- Во «Входящем сервере» `InputServerPanel.vue` использует тот же тип поля для локального и LAN URL веб-формы; при недоступном LAN остаётся пояснение и блокируется копирование.
- DSP: `src/components/audio/dsp-shared.css`. В `AudioPanel.vue` есть также **глобальный** блок `<style>`: проверять его влияние на совпадающие классы других панелей.
- Отдельные окна имеют собственные entrypoints: наличие стиля в главном окне не доказывает его наличие в них.
- Владельцы настроек и runtime-состояний: `src/composables/`; контракты — `src/types/settings.ts`, `src/ipc/`; переводы — `src/i18n/`.

При добавлении/переносе экрана обновлять соответствующую строку. Перед правкой
найти фактический класс и условия показа в исходнике; номера строк в карте не
фиксируются, поскольку быстро устаревают. Ограничить разрешённые файлы в task
по [AI workflow](./ai-workflow.md).

## Восстановление настроек

| Поверхность | Исходники | Условие открытия |
|---|---|---|
| Окно восстановления настроек | `src-settings-recovery/main.ts` → `src-settings-recovery/SettingsRecoveryApp.vue`; backend: `src-tauri/src/settings_recovery.rs` | Ошибка загрузки `settings.json` (ROADMAP-123) |

Окно описано в `src-tauri/tauri.conf.json`, доступы — в
`src-tauri/capabilities/settings-recovery.json`.
