# Дорожная карта TTSBard

Если возникли проблемы:

- **Silero:** при ошибке неподдерживаемого формата смените формат ответа бота на MP3: отправьте `/mp3` в чат с [@silero_voice_bot](https://t.me/silero_voice_bot) и повторите озвучку.
- **Поверх игры:** некоторые игры мешают показу окна или работе горячих клавиш без повышения прав. Если это происходит (например, в Marvel Rivals), запустите TTSBard **от имени администратора**.

Подробнее — в [FAQ](../ru/faq.md).

Этот раздел описывает путь развития продукта. Подробные roadmap items
распределены по фактическому состоянию:

- [`active/`](./active/) — исследуемые, запланированные, выполняемые и
  отложенные направления;
- [`completed/`](./completed/) — завершённые направления с полезным outcome;
- [`rejected/`](./rejected/) — осознанно отклонённые направления с причиной и
  условиями возможного пересмотра.

## Формат roadmap item

Каждый item начинается с canonical YAML front matter:

```yaml
---
id: ROADMAP-047
status: completed
created: 2026-07-24
updated: 2026-07-25
related_tasks: []
---
```

Номер в `id` совпадает с трёхзначным префиксом имени файла. `related_tasks`
содержит только идентификаторы долговечных задач вида `TASK-NNN`.

- В `active/` допустимы `exploring`, `planned`, `in_progress` и `deferred`.
- В `completed/` допустимы `completed` и `superseded`; завершённый item содержит
  непустой раздел `Outcome`.
- В `rejected/` допустимы `rejected` и `superseded`; отклонённый item содержит
  раздел `Reconsider when`.
- `superseded` item обязательно ссылается на заменивший roadmap item или
  decision.

Формат и соответствие каталога статусу проверяет `scripts/check-docs.ps1`.

## Активные направления

Сейчас активных направлений нет. См. [индекс](./active/README.md).

## Завершённые направления

- [ROADMAP-136 — Локальные модели RUAccent, OCR и Pocket TTS](./completed/136-local-model-management.md) — каталог из репозитория, установка и обновление пакетов.

- [ROADMAP-135 — Локальный Pocket TTS с выбором модели](./completed/135-pocket-tts.md) — локальный синтез на CPU, выбор модели и голоса.

- [ROADMAP-134 — Отключаемое преобразование чисел в слова](./completed/134-number-conversion-toggle.md) — настройка редактора, включена по умолчанию.

- [ROADMAP-133 — Настройка времени ожидания ответа Silero в интерфейсе](./completed/133-silero-synthesis-timeout-ui.md) — completed: дефолт 60 с, снятие верхнего лимита, шестерёнка [⚙] в карточке Silero и поле ввода секунд с автосохранением.

- [ROADMAP-132 — Пользовательская документация на английском и русском](./completed/132-bilingual-user-documentation.md) — `completed`: двуязычный README, 26 языковых пар, PDF EN/RU и постоянная сборка; changelog и инженерные документы остаются русскими.

- [ROADMAP-131 — Режим отправки через Twitch API](./completed/131-twitch-api.md) — `completed`: Device Code Flow для бота и каналов, список получателей и защищённое хранение; закрыто пользователем 2026-10-10.

- [ROADMAP-130 — Вывод текста и индикатора набора в VRChat](./completed/130-vrchat-chatbox-output.md) — `completed`: OSC-интеграция реализована; пользователь подтвердил ручную проверку и закрыл направление 2026-10-08.

- [ROADMAP-129 — Режим «Моно»](./completed/129-ultracompact-input-mode.md) — `completed`: реализован и закрыт пользователем 2026-10-07; ограничения native-проверки сохранены в документе.

- [ROADMAP-128 — Растягивание компактного окна](./completed/128-compact-window-resizing.md) — `completed`: размер до 800 × 630 с сохранением обеих осей; принято пользователем 2026-10-06.

- [ROADMAP-127 — Упрощение панели WebView и локального подключения](./completed/127-webview-panel-simplification.md) — `completed`: реализовано и принято пользователем после ручной проверки.

- [ROADMAP-126 — Выбор экрана захвата OCR](./completed/126-ocr-monitor-selection.md) — `completed`: реализовано и принято пользователем после ручной проверки.

- [ROADMAP-125 — Перехват клавиш и снятие ограничения](./completed/125-intercept-key-restriction.md) — `completed`: расширенный выбор клавиш и обратная связь при занятой привязке.

- [ROADMAP-124 — Согласованное управление жизненным циклом интеграций](./completed/124-integration-lifecycle-consistency.md) — `completed`: закрыто пользователем 2026-10-06.

- [ROADMAP-121 — Согласованность оформления UI](./completed/121-ui-visual-consistency.md) — `completed`: текущий UI-пакет принят; отложенный объём указан в Outcome.

- [ROADMAP-123 — Восстановление после ошибки загрузки настроек](./completed/123-settings-load-recovery.md) — `completed`.

- [ROADMAP-122 — Сохранение настроек при запуске интеграций](./completed/122-integration-start-save-feedback.md) — `completed`.

- [ROADMAP-103 — Сгруппированные панели настроек](./completed/103-grouped-general-settings.md)
- [ROADMAP-096 — Английский интерфейс и внешние языковые пакеты](./completed/096-interface-localization-and-language-packs.md)

### Ввод текста и OCR

- [ROADMAP-118 — Доступ к Input Server из локальной сети с токеном](./completed/118-input-server-lan-token-access.md)
- [ROADMAP-108 — Локальная веб-форма входящего текста](./completed/108-local-web-input-form.md)
- [ROADMAP-088 — One-shot OCR выделенной области](./completed/088-one-shot-screen-ocr.md)
- [ROADMAP-090 — Единое поведение моделей OCR и RUAccent](./completed/090-ocr-ruaccent-model-panel-behavior.md)

### Окна и запуск приложения

- [ROADMAP-100 — Согласованные поверхности пользовательского фона](./completed/100-custom-background-surface-tinting.md)
- [ROADMAP-092 — Одна копия приложения и понятное сворачивание окна](./completed/092-single-instance-and-window-lifecycle.md)

### Интеграции

- [ROADMAP-114 — Отзывчивый WebView lifecycle и устойчивый SSE](./completed/114-webview-transport-resilience.md)

- [ROADMAP-111 — Стабилизация подключения Twitch](./completed/111-twitch-connection-stabilization.md)

- [ROADMAP-107 — Текст внешней доставки для WebView и Twitch](./completed/107-original-text-for-external-delivery.md)
- [ROADMAP-106 — Ограничения длины текста TTS и внешних каналов](./completed/106-provider-text-length-limits.md)
- [ROADMAP-105 — Пользовательское тестовое сообщение Twitch](./completed/105-twitch-custom-test-message.md)
- [ROADMAP-104 — Надёжный Twitch-клиент и доставка сообщений](./completed/104-twitch-chat-client-and-delivery-resilience.md)
- [ROADMAP-101 — Маршрутизация входящих и единая вкладка редактирования](./completed/101-incoming-routing-and-edit-buffer.md)
- [ROADMAP-095 — Устойчивое переподключение Twitch IRC](./completed/095-twitch-irc-reconnect-resilience.md)
- [ROADMAP-087 — входной сервер текста для audio-only озвучивания](./completed/087-external-text-input-server.md)

### TTS-провайдеры

- [ROADMAP-119 — Готовность Piper до загрузки модели](./completed/119-piper-runtime-readiness.md) — `completed`.

- [ROADMAP-097 — ElevenLabs как TTS-провайдер](./completed/097-elevenlabs-tts-provider.md)
- [ROADMAP-086 — управляемая видимость провайдеров в панели TTS](./completed/086-tts-provider-panel-visibility.md)

### Надёжность и ревью

- [ROADMAP-117 — Разделение данных приложения: Roaming, Local и %TEMP%](./completed/117-app-storage-split.md) — `completed`.

- [ROADMAP-115 — Последовательное сохранение настроек WebView и VTube Studio](./completed/115-settings-save-consistency-followup.md)

- [ROADMAP-113 — Согласованное сохранение настроек Input Server и Twitch](./completed/113-settings-save-consistency.md)
- [ROADMAP-112 — Ограниченная очередь речи и неблокирующая запись истории](./completed/112-speech-queue-retention-and-history-io.md)
- [ROADMAP-091 — Подсказка о формате аудио Silero](./completed/091-silero-audio-format-error.md)
- [ROADMAP-089 — remediation полного ревью 2026-09-02](./completed/089-full-review-2026-09-02-remediation.md)
- [ROADMAP-084 — Ошибки очереди речи: Silero и глобальное уведомление](./completed/084-speech-queue-global-error-toast.md)
- [ROADMAP-076 — remediation по release-review v0.21.0..HEAD](./completed/076-release-review-remediation.md)

### Редактор и история

- [ROADMAP-110 — Русский словарь орфографии внутри exe](./completed/110-embedded-russian-spellcheck-dictionary.md)
- [ROADMAP-102 — Чистый список и управляемая видимость автодополнения](./completed/102-editor-autocomplete-visibility.md)
- [ROADMAP-094 — Полный системный каталог шрифтов редактора](./completed/094-system-font-catalog.md)
- [ROADMAP-093 — Шрифт и размер текста редактора](./completed/093-editor-font-settings.md)

- [ROADMAP-085 — Provider text и чистая вставка из истории фраз](./completed/085-provider-text-and-clean-history-insert.md)

- [ROADMAP-080 — сохранение текста редактора после отправки](./completed/080-keep-text-after-submit.md)
- [ROADMAP-078 — горячие клавиши режимов редактора](./completed/078-editor-hotkeys-for-modes.md)
- [ROADMAP-075 — полировка action bar и titlebar](./completed/075-editor-action-bar-and-titlebar-polish.md)
- [ROADMAP-074 — статусы интеграций в titlebar](./completed/074-ambient-integration-status.md)
- [ROADMAP-073 — маршрут фразы и результат доставки](./completed/073-readable-message-routing-and-delivery-outcomes.md)
- [ROADMAP-071 — отправка редактора и читаемый быстрый режим](./completed/071-editor-submit-and-quick-mode-affordance.md)

- [ROADMAP-068 — локальные горячие клавиши редактора](./completed/068-editor-scoped-hotkeys-and-submit-flow.md)

- [ROADMAP-001 — CodeMirror](./completed/001-monaco-vs-codemirror-editor-research.md)
- [ROADMAP-002 — локальная история и autocomplete](./completed/002-local-history-autocomplete.md)
- [ROADMAP-003 — гибридное text completion](./completed/003-text-completion-without-ai.md)
- [ROADMAP-005 — phrase history](./completed/005-phrase-history.md)
- [ROADMAP-006 — editor tabs](./completed/006-editor-tabs-multiple-texts.md)
- [ROADMAP-007 — editor menu](./completed/007-editor-menu-ai-history-spellcheck.md)
- [ROADMAP-008 — offline spellcheck](./completed/008-offline-spellcheck-hunspell-codemirror.md)
- [ROADMAP-067 — контекстное исправление орфографии](./completed/067-editor-spellcheck-context-menu.md)
- [ROADMAP-012 — persistence вкладок](./completed/012-editor-tabs-persistence.md)
- [ROADMAP-027 — layout, history и export](./completed/027-text-editor-layout-history-and-export.md)
- [ROADMAP-033 — phrase audio cache](./completed/033-phrase-history-audio-cache.md)
- [ROADMAP-034 — Silero metadata и recent dedup](./completed/034-silero-voice-and-playback-recent-dedup.md)
- [ROADMAP-048 — Enter/Escape и autocomplete](./completed/048-editor-autocomplete-enter-escape.md)
- [ROADMAP-049 — Надёжная корреляция ответов Silero](./completed/049-silero-response-correlation.md)
- [ROADMAP-052 — Перехват ошибки лимита Silero](./completed/052-silero-limit-error-handling.md)
- [ROADMAP-053 — Настройка ожидания и повторов Silero](./completed/053-silero-runtime-tuning.md)

### Окна, ввод и sound panel

- [ROADMAP-070 — маршрутизация и очередь SoundPanel](./completed/070-soundpanel-audio-routing-and-queue.md)
- [ROADMAP-072 — удаление legacy-панели Playback](./completed/072-remove-legacy-playback-panel.md)
- [ROADMAP-004 — playback control](./completed/004-playback-control-floating-window.md)
- [ROADMAP-009 — playback window architecture](./completed/009-playback-window-architecture-analysis.md)
- [ROADMAP-010 — playback window settings](./completed/010-playback-window-settings-analysis.md)
- [ROADMAP-011 — keyboard input redesign](./completed/011-keyboard-input-mechanism-redesign.md)
- [ROADMAP-014 — soundpanel sets](./completed/014-soundpanel-sets-and-inline-editing.md)
- [ROADMAP-023 — transparency и appearance](./completed/023-window-transparency-and-unified-appearance.md)
- [ROADMAP-026 — compact appearance](./completed/026-main-window-compact-appearance.md)
- [ROADMAP-037 — Hotkeys и возврат фокуса](./completed/037-application-hotkeys-and-previous-window-focus.md)
- [ROADMAP-051 — вызов плавающих окон мышью](./completed/051-mouse-access-to-floating-windows.md)
- [ROADMAP-064 — интерактивный постоянный режим SoundPanel](./completed/064-interactive-persistent-soundpanel.md)
- [ROADMAP-065 — Саундпанель: раскладка клавиатуры, разделение runtime/config](./completed/065-soundpanel-keyboard-layout-runtime-config.md)

### Audio и playback pipeline

- [ROADMAP-116 — Нагрузка audiodg и lifecycle потоков вывода](./completed/116-audiodg-playback-stream-lifecycle.md)

- [ROADMAP-109 — Качество вывода через THX Spatial Audio](./completed/109-thx-output-format-compatibility.md)

- [ROADMAP-020 — DeepFilterNet](./completed/020-audio-cleaning-enhancement.md)
- [ROADMAP-021 — resampling optimization](./completed/021-audio-pipeline-resampling-optimization.md)
- [ROADMAP-022 — effects navigation и preview](./completed/022-audio-effects-navigation-and-preview.md)
- [ROADMAP-024 — Signalsmith Stretch](./completed/024-signalsmith-stretch-audio-effects.md)
- [ROADMAP-025 — PCM playback pipeline](./completed/025-playback-pcm-pipeline.md)
- [ROADMAP-029 — DSP postprocessing](./completed/029-dsp-audio-postprocessing.md)
- [ROADMAP-030 — sample-rate invariant](./completed/030-deepfilternet-resampling-invariant.md)
- [ROADMAP-031 — Resemble Enhance research](./completed/031-resemble-enhance-research.md)
- [ROADMAP-032 — audio boundaries и presets](./completed/032-audio-boundaries-and-dsp-presets.md)
- [ROADMAP-035 — AudioPanel decomposition](./completed/035-audio-panel-subpanels.md)
- [ROADMAP-066 — Переработка UI эффектов и DSP](./completed/066-audio-effects-dsp-ui-redesign.md)
- [ROADMAP-047 — очередь задач озвучивания](./completed/047-speech-job-queue.md)
- [ROADMAP-050 — Единый список управления воспроизведением](./completed/050-unified-playback-activity-list.md)

### Архитектура, AI и документация

- [ROADMAP-099 — Notices непосредственно включённых сторонних компонентов](./completed/099-bundled-third-party-notices.md)
- [ROADMAP-098 — Снижение стоимости безопасных AI-изменений](./completed/098-ai-ready-remediation.md)
- [ROADMAP-083 — модели RUAccent и UI загрузки](./completed/083-ruaccent-omograph-model-selection.md)
- [ROADMAP-082 — нативный RUAccent runtime через ruaccent-rs](./completed/082-native-ruaccent-rs-runtime.md)
- [ROADMAP-015 — AI feature map](./completed/015-ai-features-map-and-token-benchmark.md)
- [ROADMAP-016 — project repositioning](./completed/016-project-repositioning.md)
- [ROADMAP-017 — documentation и presentation](./completed/017-documentation-and-streamer-presentation.md)
- [ROADMAP-018 — runtime architecture и AppState](./completed/018-runtime-architecture-and-appstate.md)
- [ROADMAP-019 — custom AI provider](./completed/019-custom-ai-provider.md)
- [ROADMAP-028 — secret-safe logging](./completed/028-secret-safe-logging.md)
- [ROADMAP-036 — Telegram auth polish](./completed/036-telegram-auth-flow-polish.md)
- [ROADMAP-038 — Dynamic Piper providers](./completed/038-dynamic-piper-tts-providers.md)
- [ROADMAP-039 — Embedded Piper runtime и лицензирование](./completed/039-piper-runtime-feasibility.md)
- [ROADMAP-040 — Test coverage gaps](./completed/040-test-coverage-gaps.md)
- [ROADMAP-041 — Review 021 remediation](./completed/041-review-021-remediation.md)
- [ROADMAP-046 — Documentation migration](./completed/046-documentation-structure-migration.md)
- [ROADMAP-055 — Качество и AI-ready foundation](./completed/055-quality-and-ai-ready-architecture.md)
- [ROADMAP-056 — IPC-контракты и AI-ready границы](./completed/056-ipc-contracts-and-ai-ready-boundaries.md)
- [ROADMAP-057 — Достоверный handoff и продуктовые источники](./completed/057-truthful-handoff-and-product-sources.md)
- [ROADMAP-058 — Исполнимые контракты и lifecycle-проверки](./completed/058-executable-contracts-and-lifecycle-proof.md)
- [ROADMAP-059 — Владение integration state и атомарность settings](./completed/059-integration-state-ownership-and-settings-atomicity.md) — P0–P3: WebView atomic save, Telegram owner API, контракты DECISION-018/019.

### VTube Studio и WebView

- [ROADMAP-120 — Подключение VTube Studio по локальной сети](./completed/120-vtube-studio-lan-connection.md) — `completed`.

- [ROADMAP-079 — затухание ошибки ручного подключения VTS](./completed/079-vts-manual-connect-error-decay.md)
- [ROADMAP-042 — VTube Studio typing UI](./completed/042-vtube-studio-typing-ui.md)
- [ROADMAP-043 — WebView typing events](./completed/043-webview-editor-typing-events.md)
- [ROADMAP-044 — VTube Studio connection lifecycle](./completed/044-vtube-studio-connection-lifecycle-ui.md)
- [ROADMAP-045 — typing output modes](./completed/045-vtube-studio-typing-output-modes.md)
- [ROADMAP-054 — Детерминированная видимость предмета VTube Studio](./completed/054-vtube-studio-item-visibility.md)
- [ROADMAP-060 — Корректный lifecycle параметра печати VTube Studio](./completed/060-vtube-studio-parameter-input-lifecycle.md)
- [ROADMAP-061 — Настройка действия и замена custom INPUT VTube Studio](./completed/061-vtube-studio-action-configuration-and-parameter-replacement.md)
- [ROADMAP-062 — Достоверное runtime-состояние соединения VTube Studio](./completed/062-vtube-studio-runtime-connection-truth.md)
- [ROADMAP-063 — Фактическое runtime-состояние WebView-сервера](./completed/063-webview-runtime-server-status.md)

### Remediation и устойчивость

- [ROADMAP-069 — Устранение дефектов полного review 0.21.0](./completed/069-full-review-correctness-remediation.md)

## Отклонённые направления

- [ROADMAP-013 — переход history storage на SQLite](./rejected/013-history-storage-json-vs-sqlite.md) — отклонён до появления объёма и конкурентного доступа, оправдывающих БД.
