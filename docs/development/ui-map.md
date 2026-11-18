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
| `input` | Ввод | `InputPanel.vue` | `editor/TtsEditor.vue`, `editor/EditorTabs.vue`, `editor/EditorMenu.vue`, `editor/RouteSelector.vue`, `editor/IncomingTextsTab.vue` (маршрут и авто-воспроизведение входящего текста), `editor/IncomingRouteSelector.vue`, `editor/SpellContextMenu.vue`, `PhraseHistoryList.vue` |
| `tts` | Синтез речи | `TtsPanel.vue` | `tts/TtsOpenAICard.vue`, `TtsSileroCard.vue`, `TtsLocalCard.vue`, `TtsPocketCard.vue`, `TtsPiperCard.vue`, `TtsFishAudioCard.vue`, `TtsElevenLabsCard.vue`, `VoiceSelector.vue`, `FishAudioModelPicker.vue`, `TelegramConnectionStatus.vue`, `TelegramAuthModal.vue` (в корне `src/components/`, вне `tts/`) |
| `audio` | Аудио | `AudioPanel.vue` | `audio/AudioDevicesTab.vue`, `AudioEffectsTab.vue`, `audio/AudioVbanTab.vue` (настройки и тест сетевого выхода), `AudioPreviewBar.vue`, `DspSettings.vue`, `EqSettings.vue`, `CompressorSettings.vue`, `LimiterSettings.vue`, `EffectsSettings.vue` |
| `preprocessor` | Препроцессор | `PreprocessorPanel.vue` | Правила обработки и их условные настройки |
| `webview` | WebView | `WebViewPanel.vue` | Сервер с постоянным LAN bind, порт, локальный/LAN URL; шаблоны, тест, внешнее подключение и UPnP |
| `twitch` | Twitch | `TwitchPanel.vue` | Режимы подключения (IRC и Twitch API), реквизиты в блоке подключения; отдельный блок «Аккаунты» для режима API: авторизация бота через Device Code Flow, локальный сброс бота с сохранением реквизитов и каналов, список авторизованных каналов с выбором одного получателя и автоматическим обновлением активного API-подключения при смене канала и локальным удалением доступа, добавление канала через Device Code Flow с копированием/открытием ссылки активации, ошибки авторизации и хранилища, тест |
| `vtube-studio` | VTube Studio | `VTubeStudioPanel.vue` | Адрес/порт, параметры и действия при наборе, тестирование |
| `vrchat` | VRChat | `VRChatPanel.vue` | Адрес и порт OSC, запуск при старте, тестовое сообщение, помощь |
| `ocr` | OCR | `OcrPanel.vue` | Настройки распознавания, выбор экрана захвата и выбор области |
| `input-server` | Входящий сервер | `InputServerPanel.vue` | Сервер, адреса веб-формы и токен, API `/v1/speech`, тест. Маршрут входящего текста и авто-воспроизведение выбираются не здесь, а на поверхности входящих — `editor/IncomingTextsTab.vue` |
| `hotkeys` | Горячие клавиши | `HotkeysPanel.vue` | Назначение сочетаний |
| `intercept` | Перехват | `InterceptPanel.vue` | Настройки перехвата |
| `settings` | Настройки | `SettingsPanel.vue` | Вкладки ниже |

## Настройки

| ID вкладки | Компонент | Связанные поверхности |
|---|---|---|
| `general` | `src/components/settings/SettingsGeneral.vue` | Язык, поведение окон, кнопки дополнительных окон (скрытие в заголовке), папки, диагностика, карточка «Обновления» (см. ниже); `DataTransferModal.vue` в том же каталоге |
| `interface` | `src/components/settings/SettingsInterface.vue` | Тема и внешний вид окон |
| `editor` | `src/components/settings/SettingsEditor.vue` | `EditorFontSettings.vue` в том же каталоге; карточка преобразования чисел сразу после шрифтов |
| `network` | `src/components/settings/SettingsNetwork.vue` | Сетевые формы |
| `models` | `src/components/settings/SettingsModels.vue` | Каталог RUAccent, OCR и Pocket TTS, установка и повторная установка, переход с панели OCR; `SettingsRuAccent.vue` для выбора и загрузки модели ударений |
| `ai` | `src/components/SettingsAiPanel.vue` | Настройки AI |

Карточка «Обновления» (секция `updates-section` в `SettingsGeneral.vue`):
заголовок на отдельной строке. Ниже текущая версия и кнопка «Проверить сейчас»,
заблокированная на время проверки, затем строки последней успешной проверки и последней
попытки. Даты стоят рядом с подписями и отображаются в локальном времени
в формате `ДД.ММ.ГГГГ ЧЧ:ММ`. Строка попытки скрыта до первой попытки и
после успешной проверки с совпадающим временем. Сразу под датами отображается
результат последней попытки. Внизу две независимые
галочки — «Проверять обновления при запуске приложения» (по умолчанию
включена) и «Проверять обновления во время работы» с селектором «Интервал проверки»
1–7 дней (по умолчанию выключена, селектор недоступен, значение
сохраняется); результат последней завершённой попытки остаётся в карточке
до следующей попытки — «Установлена актуальная версия», постоянная плашка
ошибки проверки (`role="status"`, без попапов и баблов) либо релиз
с Markdown-описанием и кнопкой GitHub в строке версии; свежая ошибка
и найденный ранее релиз показываются одновременно, успех снимает прежнюю
ошибку. Кнопка «Обновить» по центру под прокручиваемым описанием — только
когда запущенное приложение само живёт в каталоге NSIS-установки
и подтверждён updater-артефакт релиза (`install_available`). Состояние
карточки — `src/composables/useAppUpdates.ts`, контракт —
`src/ipc/appUpdates.ts`, настройки, расписание и результат —
`src-tauri/src/updates/mod.rs` и `src-tauri/src/commands/updates.rs`.

## Общие поверхности и отдельные окна

| Поверхность | Источник |
|---|---|
| Titlebar и переключение дополнительных окон | `src/App.vue`, `titlebar/OutputControlsGroup.vue` (общие аудиовыходы, ROADMAP-142) |
| Статусы интеграций | `src/components/titlebar/IntegrationStatusCluster.vue`, проекция WebView, Twitch, VTube Studio, VRChat (включённый исходящий OSC) и входящего сервера — `src/composables/useIntegrationStatusSlots.ts` |
| Уведомления | Глобальная стопка в `src/components/ErrorToasts.vue`, панельные всплывающие сообщения в компонентах панелей, встроенные статусы в `src/components/shared/StatusMessage.vue` и `src/components/shared/TestResult.vue`. [Правила всплывающих уведомлений](ui-guidelines.md#всплывающие-уведомления) |
| Установка обновления | `src/components/settings/UpdateInstallModal.vue` — модальное окно прогресса установки (проверка → загрузка → сохранение → запуск установщика). Загрузку отменяют «Отмена» и крестик. Escape и клик по подложке во время загрузки игнорируются. Ошибка предлагает «Повторить»/«Закрыть». Хостится в `App.vue` через `src/composables/useUpdateInstall.ts`, чтобы прогресс не терялся при уходе из настроек; запуск — кнопка «Обновить» в карточке «Обновления». Состояние — `src/composables/useUpdateInstall.ts`, контракт — `src/ipc/appUpdates.ts`, конвейер — `src-tauri/src/updates/install.rs` |
| Бабл обновления | `src/App.vue` — нейтральное уведомление в стиле системных тостов у правого края под заголовком обычного и компактного окна или под панелью «Моно». Иконка информации и общий крестик `ui-notice-close`. «Подробнее» из компактного режима и «Моно» восстанавливает обычное окно и открывает «Настройки → Общие → Обновления». Крестик скрывает уведомление для этой версии между запусками. Без таймера. Состояние — `src/composables/useAppUpdates.ts`, контракт — `src/ipc/appUpdates.ts`, фоновая проверка — `src-tauri/src/updates/mod.rs` |
| Компактный режим | `src/composables/mainWindowMode.ts` (единый владелец), `src/composables/mainWindowModeAdapter.ts` (native IPC), `src/components/MinimalModeButton.vue` (кнопка), `src/composables/useCompactWindowResize.ts`, `src/components/InputPanel.vue` — изменение размера за края окна; внутренние захваты в компактном режиме отсутствуют, разделитель высоты редактора доступен в обычном режиме. Меню `EditorMenu.vue` позволяет перейти в моно из компактного и обычного окна, а также независимо скрыть/показать панель вкладок; предпочтение видимости хранится в localStorage (`app-editor-tabs-visible`), сохраняется при выходе из моно. На поверхности входящих вкладки остаются доступны для возврата к тексту. При скрытых вкладках редактор имеет скругление всех углов 6 px. Список маршрутов в нижней панели раскрывается вверх |
| Режим «Моно» | `src/components/MonoModeBar.vue`, `src/composables/mainWindowMode.ts`, `src/composables/useIntegrationStatusSlots.ts`, `src/App.vue`, `src/components/InputPanel.vue` — стиль компактного режима: скрыты titlebar, вкладки, action bar и история. Угловая `MinimalModeButton` остаётся для прямого возврата в обычное окно. Сверху отображается `MonoModeBar` (18 px) с центральной ручкой, кнопкой `ChevronDown` для возврата в компактный вид, кнопкой сворачивания окна и значками только активных ошибок интеграций с popover подробностей без смещения редактора или расширения окна. При переходе между компактным видом и моно геометрия окна сохраняется без ресайза; переход из обычного окна применяет сохранённый компактный размер |
| Звуковая панель | `src-soundpanel/main.ts` → `SoundPanelApp.vue` в том же каталоге. Отдельной вкладки SoundPanel в главном окне нет; `src/composables/useSoundPanel.ts` — legacy-композабл, не подключённый ни к одному компоненту и дублирующий типы/IPC `SoundPanelApp.vue` |
| Управление воспроизведением | `src-playback/main.ts` → `PlaybackControlApp.vue` в том же каталоге |
| Выбор области OCR | `src-ocr-selection/main.ts` → `SelectionApp.vue` в том же каталоге |
| Браузерный overlay | `src-tauri/src/input_server/overlay.html` |

## Где искать оформление и поведение

- Главное окно: `src/style.css` и `src/styles/index.css`; цвета и переменные — `src/styles/variables.css`; базовые стили — `base.css`; общий фокус — `focus.css` в `src/styles/`.
- Согласованные размеры и отступы основных форм — `src/styles/ui-tokens.css`; общий слой явно подключаемых классов `ui-*` — `src/styles/ui-controls.css`. Переведены на слой: вкладки настроек и аудио, все вкладки настроек, TTS-карточки и диалоги, OCR, препроцессор, перехват и горячие клавиши (обычные роли), аудио-устройства/эффекты (обычные роли), интеграции, шрифтовые пикеры. Компактные поверхности редактора/истории, титлбар и отдельные окна сохраняют собственную плотность (предложение шкалы — в ROADMAP-121); глобальный `<style>` в `AudioPanel.vue` остаётся владельцем стилей плотных DSP-подсекций.
- Общие контролы: `src/components/shared/InputWithToggle.vue`, `ProviderCard.vue`, `StatusMessage.vue`, `TestResult.vue` в том же каталоге.
- Редактор текста: `editor/TtsEditor.vue` — нижний радиус 6 px; в обычном режиме фокус обозначается приглушённой границей без свечения. В компактном режиме и «Моно» `InputPanel.vue` полностью убирает оформление фокуса редактора, включая клавиатурный фокус; каретка и выделение сохраняются. В «Моно» поле постоянно обозначено нейтральной рамкой 1 px (`rgba(var(--rgb-border), 0.22)`) по всем сторонам, одинаковой при фокусе и без него.
- Аудио: обновление списка устройств находится справа от вкладок в `AudioPanel.vue`, вне `tablist`, и показывается только на вкладке «Устройства». Загрузка и обновление списка остаются в `audio/AudioDevicesTab.vue`; во время обновления кнопка заблокирована и показывает spinner. Вкладки могут переноситься внутри оставшейся ширины строки. Форма VBAN находится в `audio/AudioVbanTab.vue`, индикатор состояния сетевого выхода на вкладке — в `AudioPanel.vue`. Глобальное предупреждение о недоступности VBAN-приёмника (включая ошибку локального порта) показывается через общий error-toast из `composables/useVbanHealthNotifications.ts`, подключённого в `App.vue`; тексты — `audio.vban.toast.*`. Общие аудиовыходы (ROADMAP-142): `titlebar/OutputControlsGroup.vue` в общей titlebar `App.vue`, normal/compact, меню вниз; `editor/OutputToggles.vue` остаётся переиспользуемым view, из InputPanel группа убрана. Источники состояния audio.virtual_mic_enabled/audio.vban.enabled, persist-first controller useOutputToggles.ts; main-window document keydown работает на любом разделе и в полях ввода, сочетания в UI-группе «Основное окно», без OS hook и без обработчиков в отдельном SoundPanel. Сериализация hotkeys.editor.toggle_* сохранена ради совместимости, старый editor dispatch удалён. Guard записи — useMainWindowOutputControls.ts и HotkeysPanel.vue. Выключение общее для речи и SoundPanel, сразу прекращает выбранный выход без ожидания очереди; настройки и другие выходы сохраняются, это не mute и не отключение драйвера Windows. Проверка устройств — useVirtualMicDevices.ts: initial/open/selected change, последний подтверждённый список сохраняется во время refresh, stale guard; ненастроенный выход disabled, причина только title/aria, без inline-подписей. AI-действия остаются в меню «…». Runtime по-прежнему playback.rs, soundpanel/audio.rs и audio/vban; перенос не меняет аудиодвижок.
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
