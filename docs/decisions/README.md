# Решения

Короткие записи об устойчивых архитектурных и продуктовых решениях. Одна запись
описывает один контекст, выбранный вариант, последствия и статус. Временные
implementation details и журналы обсуждений сюда не помещаются.

Статусы: `accepted`, `superseded`, `deprecated`.

## Действующие решения

- [DECISION-001 — Windows platform](./001-windows-platform.md)
- [DECISION-002 — Tauri application shell](./002-tauri-shell.md)
- [DECISION-003 — Commands и typed events](./003-commands-and-events.md)
- [DECISION-004 — Service-owned state](./004-service-owned-state.md)
- [DECISION-005 — Registry TTS-провайдеров](./005-tts-provider-registry.md)
- [DECISION-006 — Единый PCM pipeline](./006-pcm-audio-pipeline.md)
- [DECISION-007 — Текстовый pipeline и AI](./007-text-pipeline-and-ai.md)
- [DECISION-008 — JSON persistence](./008-json-persistence.md)
- [DECISION-009 — Окна и хоткеи](./009-windows-and-hotkeys.md)
- [DECISION-010 — WebView/SSE server](./010-webview-sse.md)
- [DECISION-011 — Lifecycle интеграций](./011-integration-lifecycle.md)
- [DECISION-012 — CSS tokens и темы](./012-css-tokens.md)
- [DECISION-013 — Сетевая конфигурация](./013-network-configuration.md)
- [DECISION-014 — Ошибки и проверка](./014-errors-and-validation.md)
- [DECISION-017 — Лицензия GPL-3.0-only](./017-gpl-3-license.md)
- [DECISION-018 — Telegram client ownership: контракт разделения Arc](./018-telegram-client-ownership.md) — почему `client` остаётся `pub(crate)` (не private) и правило доступа `lock → clone → drop → await`
- [DECISION-019 — Контракт settings-локов integration-сервисов](./019-integration-settings-arc-contract.md) — Twitch/WebView/VTube `settings: Arc<RwLock<_>>` остаётся публичным; owner-API не нужен (нет дефектов)
- [DECISION-020 — Настройка действия VTube Studio требует живого подключения](./020-vtube-action-requires-live-connection.md) — `save_vtube_studio_typing_action` отклоняется без живого соединения (отменяет ROADMAP-060 P0.4); переименование = create→persist→delete
- [DECISION-021 — Захват виртуального рабочего стола через xcap](./021-ocr-virtual-desktop-capture-xcap.md) — one-shot OCR снимает весь виртуальный стол до UI; геометрия/кроп — чистые функции
- [DECISION-022 — Twitch IRC lifecycle принадлежит twitch-irc](./022-twitch-irc-lifecycle.md) — библиотека владеет transport/reconnect/JOIN; приложение — статусом, очередью и семантикой `sent`
- [DECISION-023 — Ограниченное хранение terminal jobs очереди речи](./023-speech-queue-terminal-retention.md) — 50 последних `Completed`/`Cancelled`, вытеснение самого старого; active и `Failed` не вытесняются
- [DECISION-024 — UPnP port forwarding вне async lifecycle WebView](./024-webview-upnp-ownership.md) — router I/O на blocking pool с bounded wait и epoch-поколениями; поздний mapping после stop/disable компенсируется; readiness listener не зависит от forwarding

- [DECISION-025 — Общее защищённое хранилище](./025-shared-secret-store.md) — общий DPAPI-файл с разделами интеграций; первым подключён Twitch.

- [DECISION-026 — Авторизация Twitch через Device Code Flow](./026-twitch-device-flow.md) — ссылка с кодом активации для бота и каналов; localhost callback не используется.

- [DECISION-027 — Локальный нейросетевой Pocket TTS и нормализация референса](./027-pocket-tts-onnx-runtime.md) — локальный ONNX Runtime на CPU (Mimi + FlowLM), режимы `reference_wav` и `voice_state`, нормализация входного референса до RMS ~0.048 и компенсация выходного гейна до RMS ~0.12.

## Заменённые решения

- [DECISION-015 — Отдельный F6-режим](./015-hardcoded-f6-mode.md)
- [DECISION-016 — Раздельные playback-пути](./016-split-playback-paths.md)
