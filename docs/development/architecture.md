# Архитектура TTSBard

Этот документ описывает устойчивые границы системы и основные потоки данных.
Детали конкретных функций, полный список модулей и версии зависимостей следует
смотреть в исходном коде, `Cargo.toml` и `package.json`.

## Контекст системы

TTSBard — Windows desktop-приложение на Tauri 2. Интерфейс работает на Vue 3,
а системные интеграции, TTS, обработка аудио и фоновые сервисы находятся в Rust.

```mermaid
flowchart LR
    UI["Vue UI"] <-->|"Tauri commands / events"| Core["Rust application core"]
    Core --> TTS["TTS providers"]
    Core --> Audio["Audio pipeline and devices"]
    Core --> OS["Windows hooks, hotkeys and windows"]
    Core --> Services["WebView, Twitch, Telegram, VTube Studio"]
    Browser["OBS / browser clients"] <-->|"HTTP + SSE"| Services
    External["External text clients"] -->|"POST /v1/speech, loopback"| Intake["Shared external-text intake"]
    Screen["Screen selection"] --> OCR["Local OCR"] --> Intake
    Intake --> Inbox["Incoming: review or auto-play"] --> Core
```

## Точки входа и запуск

- `src/main.ts` монтирует Vue-приложение из `src/App.vue`.
- `src-tauri/src/main.rs` передаёт управление библиотечному entry point.
- `src-tauri/src/lib.rs::run` создаёт Tauri builder, регистрирует shared state и
  команды frontend API.
- `src-tauri/src/setup.rs::init_app` связывает настройки, окна, event channels,
  keyboard hooks, playback и фоновые интеграции. Кроме wiring, `setup.rs` сейчас
  содержит долгоживущие runtime-циклы (в частности, speech worker).

`lib.rs` и `setup.rs` являются composition root. Доменные правила не следует
добавлять туда, если у них есть подходящий сервис или модуль.
`speech_queue.rs` выполняет задания очереди речи, `shutdown.rs` координирует
выход из приложения. `commands/mod.rs` содержит общие операции IPC,
`commands/tts_pipeline.rs` готовит текст и аудио для синтеза,
`commands/window.rs` управляет окнами, внешним видом и хоткеями.

## Основные границы

| Область | Владелец | Назначение |
|---|---|---|
| UI | `src/components/`, `src/composables/` | Представление, формы и локальное состояние интерфейса |
| Команды | `src-tauri/src/commands/` | Граница вызовов Vue → Rust, подготовка речи, управление окнами и хоткеями |
| Завершение работы | `shutdown.rs` | Сохранение вкладок, отмена фоновых задач и остановка сервисов перед выходом |
| События | `events.rs`, `event_loop.rs` | Внутреннее распределение событий и уведомления окон |
| Состояние приложения | `state.rs` | Общие runtime handles, сервисы, registry и hot-path state |
| Настройки | `config/` | Валидация, cache и сохранение пользовательских настроек |
| TTS | `tts/` | Провайдеры, их registry и единый контракт синтеза |
| Аудио | `audio/`, `signalsmith/`, `playback.rs` | Декодирование, DSP, очередь и вывод на устройства |
| Редактор | `editor.rs`, `preprocessor/`, `spellcheck.rs`, `history.rs`, `tabs.rs`, `stress/` | Подготовка текста, история ввода и фраз, вкладки редактора, проверка орфографии, расстановка ударений RUAccent |
| Окна и ввод | `window.rs`, `commands/window.rs`, `hotkeys.rs`, `config/windows.rs`, `playback_window.rs`, `soundpanel_window.rs`, `soundpanel/` | Окна, их геометрия и внешность, глобальные клавиши и низкоуровневый ввод |
| Интеграции | `webview/`, `twitch/`, `telegram/`, `vtube_studio/` | Внешние протоколы и lifecycle подключений |
| Внешний текст | `input_server/`, `commands/input_server.rs` | Loopback listener, общий intake, очередь ожидающих реплик |
| OCR | `ocr/`, `commands/ocr.rs` | Пакеты моделей, runtime, захват, распознавание и передача в intake |
| Локальные модели | `model_downloads.rs`, `catalog/model-catalog.json` | Каталог пакетов, загрузка и установка моделей, проверка хешей |

## Frontend ↔ backend

Frontend вызывает Rust через Tauri commands. Команда должна:

1. разобрать и провалидировать входные данные;
2. передать работу доменному сервису;
3. вернуть сериализуемый результат или безопасную ошибку.

Долгоживущие изменения состояния backend сообщает окнам через Tauri events.
Внутренний `AppEvent` не является публичным frontend-контрактом: его принимает
`EventHandler`, который обновляет backend и при необходимости рассылает внешние
события.

## Владение состоянием

`AppState` — основной runtime-контейнер. В нём находятся общие сервисы WebView,
Twitch, VTube Studio и editor, TTS registry, единый Tokio runtime, cancellation
token, кэши и небольшие runtime-флаги.

`AppState` также владеет `InputServerService` и `OcrService`. Общая политика
«Входящих» и ожидающие элементы пока принадлежат `InputServerService`, хотя
используются и OCR без запущенного HTTP-listener.

Отдельные Tauri-managed состояния используются там, где жизненный цикл уже
выделен из `AppState`: настройки, окна, playback, SoundPanel, история, вкладки и
Telegram.

Инварианты владения:

- доменный service владеет своими settings/status и предоставляет операции над
  ними;
- persisted settings проходят через `config/`, а не записываются компонентами
  напрямую;
- новые async-задачи используют общий runtime и общий shutdown token;
- `coordinate_shutdown` — единственная идемпотентная точка выхода: sidebar,
  tray и OS exit проходят через неё до остановки сервисов;
- mutex/RwLock guard нельзя удерживать во время сетевого запроса, аудиооперации
  или другого длительного `await`;
- UI не считается источником истины для состояния фонового сервиса.

## Основные потоки

### Синтез и воспроизведение

```text
InputPanel
  → Tauri command submit_speech
  → immutable SpeechSnapshot
  → SpeechQueue
  → speech worker
  → text preprocessing и prefix flags
  → выбранный provider из TtsProviderRegistry
  → audio decode / resample / DSP
  → PlaybackManager с per-phrase output config
  → выбранные audio devices
  → playback events в UI
```

Провайдер возвращает аудиоданные через общий TTS-контракт. Провайдер не должен
самостоятельно управлять окнами или очередью воспроизведения.

Поддерживаются следующие категории провайдеров (`src-tauri/src/tts/`):
- **Удалённые сервисы:** OpenAI TTS, ElevenLabs, Fish Audio, Silero Bot (через Telegram-клиент).
- **Локальный HTTP:** подключение внешнего совместимого веб-сервера (например, TITTS.py).
- **Встроенный Piper:** локальный синтез через C++ runtime Piper и фонемизатор `espeak-ng`.
- **Встроенный Pocket TTS (`tts/pocket/`):** локальный нейросетевой синтез на CPU
  через ONNX Runtime (архитектура Kyutai: кодек `mimi_encoder`/`mimi_decoder` 24 кГц
  и авторегрессионный `FlowLM` с INT8-квантизацией). Поддерживает клонирование
  голоса по референсному WAV и встроенные голоса (`.safetensors`). Включает
  алгоритмическую подготовку/нормализацию входного референса (активный RMS ~0.048)
  для стабилизации базовой частоты F0 и автоматическую компенсацию выходного
  гейна (целевой RMS ~0.12).

Правила эффектов и частоты дискретизации описаны в [аудиодокументации](../user/audio-effects.md).

Общий формат выходного потока хранится в `audio.output_format` (`default | i32`)
и читается из разделяемого кэша `SettingsManager` при открытии каждого потока.
Отсутствующее поле старой конфигурации означает default. PCM, эффекты и микшер
остаются f32; strict i32 преобразует сэмплы в CPAL callback без fallback.
Открытые потоки удерживаются `OutputDeviceStream` и не пересоздаются при смене
настройки. Их счётчики по выбранному режиму обеспечивают статус отложенного
применения, в том числе для preview, теста и SoundPanel. UI использует обычное
событие `settings-changed` после успешного сохранения; переменная окружения
`TTSBARD_OUTPUT_FORMAT` и отдельный session override больше не используются.
Старый путь глобального перехвата текста и `AppEvent::TextReady` удалён; low-level
keyboard hook используется только экспериментальным Numpad/F-key control для
дискретных действий приложения.

### Внешний текст и OCR

HTTP `POST /v1/speech` и результат OCR сходятся в
`commands/input_server.rs::accept_external_text`. Общий intake валидирует
вход и применяет политику `IncomingSettings`: принять задание в `SpeechQueue`
либо сохранить элемент для ручного разбора. OCR выполняет локальный захват и
распознавание, после чего передаёт текст с отметкой источника OCR.

Для автоматически принятой реплики создаётся snapshot провайдера и голоса.
Далее используется общий speech pipeline. Delivery policy задаёт persisted
маршрут входящего текста `IncomingRoute`: `AudioOnly`, `AudioWebview`,
`AudioTwitch` или `Everywhere`. Аудио присутствует во всех маршрутах, а
рассылка в WebView и Twitch включается выбранным маршрутом; неизвестное или
отсутствующее значение нормализуется в `AudioOnly`. Действие «Редактировать»
открывает обычную вкладку:
последующая отправка подчиняется уже маршруту редактора.

Frontend показывает вкладку «Входящие», если HTTP-сервер работает, OCR включён
в настройках или есть ожидающие элементы (`InputPanel.vue::incomingAvailable`).
Это условие доступности вкладки, а не подтверждение готовности OCR runtime.
Контракты — [HTTP](../user/input-server.md), пользовательский разбор —
[«Входящие»](../user/incoming.md), установка моделей — [OCR](../user/ocr.md).

### Рассылка текста и typing state

После принятия текста backend публикует типизированные события. WebView и Twitch
получают текст независимо; prefix flags могут отключить конкретного consumer.
Typing state также раздаётся отдельным consumers WebView и VTube Studio, чтобы
сбой одной интеграции не блокировал другую.

Контракты браузерного клиента описаны в [WebView](../integrations/webview.md) и
[SSE](../integrations/sse.md).

### SoundPanel

SoundPanel — отдельное Tauri/WebView-окно, а не панель главного frontend:

- `src-soundpanel/SoundPanelApp.vue` владеет QWERTY-сеткой, локальным keydown,
  режимами runtime/config и UI слоёв;
- `soundpanel/bindings.rs` предоставляет IPC для слоёв, назначений, запуска
  звука, Escape, appearance и закрепления;
- `SoundPanelState` хранит runtime-наборы, active layer, runtime-копию
  persisted `stay_visible`, config-mode, transient focused flag и bounded
  runtime-only FIFO-очередь звуков;
- `soundpanel_window.rs` владеет показом, скрытием, возвратом foreground HWND и
  отложенным blur-hide;
- low-level hook в `soundpanel/hook.rs` обслуживает глобальный Intercept. Когда
  SoundPanel в фокусе, F1–F12 пропускаются в DOM для локального выбора слоя;
  вне активной панели Intercept сохраняет обычное глобальное поведение.

Слои и назначения сохраняются в `soundpanel_bindings.json`. Геометрия,
appearance и закрепление находятся в `windows.json`. Пользовательский pin —
единый source of truth: `stay_visible=true` означает, что blur, Escape и запуск
звука не скрывают окно; legacy `hide_on_blur` нормализуется как обратный флаг.

Очередь SoundPanel запускается в `setup::init_app` и останавливается общим
shutdown token. Каждый её элемент несёт snapshot `AudioSettings`: звук
отправляется в включённые динамики и настроенный виртуальный микрофон через
общий resolver устройств. Один worker завершает текущий звук на всех доступных
выходах до начала следующего. Этот путь намеренно независим от `SpeechQueue`
и `PlaybackManager`, поэтому эффекты панели не накладываются друг на друга,
но не перебивают и не задерживают TTS.

Пользовательский контракт описан в [руководстве по SoundPanel](../user/soundpanel.md),
а эволюция сетки и режимов — в [ROADMAP-065](../roadmap/completed/065-soundpanel-keyboard-layout-runtime-config.md).

### Запуск и завершение

`setup.rs` восстанавливает настройки, создаёт каналы и запускает workers.
Синхронный main event loop работает в выделенном потоке, async-интеграции — на
общем Tokio runtime. При завершении cancellation token должен остановить
фоновые сервисы до уничтожения runtime и окон. Все пользовательские и системные
пути выхода сходятся в `shutdown::coordinate_shutdown`. Повторный запрос
безопасно игнорируется после начала shutdown.

### Persistence и blocking boundaries

Изменение persisted state строит candidate, записывает его через общий
Windows-safe atomic replace и публикует cache/runtime только после успешной
записи. Составные изменения (SoundPanel binding и Fish Audio connection form)
сохраняются одной owner-level transaction. Настройки перехвата SoundPanel
(`InterceptSettings`) разделяют readers и writers: `SoundPanelState::get_intercept`
читает короткий runtime lock напрямую, поэтому low-level keyboard hook никогда
не блокируется на диск; мутации сериализуются через общий writer gate,
сохраняются на диск вне runtime lock и обновляют runtime со строгим порядком
событий до освобождения gate. Файловые операции, cache/decode/DSP
и другие синхронные тяжёлые этапы speech pipeline запускаются через явную
blocking isolation, а не на общем async executor.

### Runtime status WebView

`WebViewSettings.enabled` — persisted desired state, а не признак занятого TCP
listener. `WebViewService` владеет runtime snapshot `Stopped`, `Starting`,
`Running` или `Error`; `Running` публикуется только после успешного bind.
Frontend сначала подписывается на `webview-server-status-changed`, затем читает
`get_webview_server_status`, поэтому не теряет переход между snapshot и listen.
Контролы отправки теста и запущенного сервера доступны только в `Running`.

## Режимы главного окна

`App.vue` создаёт один `mainWindowMode` controller и передаёт его потомкам
через типизированный `MAIN_WINDOW_MODE_KEY`. Controller владеет текущим режимом
`ordinary | compact | mono`, запомненным компактным видом и подтверждёнными
компактными размерами. Компоненты читают производное состояние и отправляют
действия; кнопка режима не выполняет IPC. `mainWindowModeAdapter` связывает
переходы с существующими native-командами bounds, resize и persistence.

`useCompactWindowResize` сохраняет владение жестами, debounce и очередями
resize/save. Он явно регистрирует flush перед выходом и использует leases
подавления сохранения служебных resize-событий. Настройки поступают controller
через hooks загрузки с revision, снятой до запроса: устаревший snapshot не
должен откатывать локальный выбор или размеры. `start_compact` определяет
следующий запуск; загрузка настройки не переключает уже открытое окно.
`compactModeState.ts` содержит только startup/cache helpers без mutable runtime.

## Concurrency

- `std::sync::mpsc` связывает producers с главным обработчиком `AppEvent`.
- SoundPanel использует отдельный FIFO-worker для воспроизведения звуков;
  он не является частью очереди речи.
- Tokio tasks обслуживают сеть и другие async-интеграции.
- `Arc<Mutex<_>>`, `Arc<RwLock<_>>` и atomics применяются только для общего
  runtime state; предпочтительнее методы владельца, чем прямой доступ к полям.
- Блокирующее аудио, файловое или Windows API не должно выполняться на async
  executor без явной изоляции.

## Безопасность

- Секреты нельзя включать в логи, ошибки frontend или документы репозитория.
- Внешние URL и пользовательские пути валидируются на границе команды/service.
- WebView server применяет собственные правила bind address и token; отдельный
  CORS layer не установлен, а встроенный template использует same-origin пути;
  см.
  [доступ из сети WebView](../integrations/webview.md#доступ-из-сети).
- Windows hooks активируются только в требуемом режиме и освобождаются при
  shutdown.
- Новая интеграция должна иметь явные connect/disconnect операции, timeout и
  безопасное состояние после ошибки.

## Куда вносить изменения

| Изменение | Начальная точка |
|---|---|
| Новый frontend action | соответствующий composable и command в `commands/` |
| Новый TTS provider | backend contract/implementation/registry в `tts/`; `config/` DTO и persistence; command + регистрация в `lib.rs`; frontend type, card/selection/visibility metadata и локализация; затем IPC/settings checks, frontend и Rust tests |
| Изменение аудиопайплайна | `audio/`, `playback.rs`, связанные roadmap/decision |
| Новое внутреннее событие | `events.rs` и исчерпывающий routing в `event_loop.rs` |
| Новая настройка | schema/validation/persistence в `config/`, затем UI |
| Новая внешняя интеграция | отдельный service с owned lifecycle и typed events |
| Изменение инициализации | `setup.rs`; `lib.rs` только для регистрации boundary |

Перед архитектурным изменением проверьте активный [roadmap](../roadmap/README.md)
и существующие [решения](../decisions/README.md). Команды проверки и сборки
находятся в [документации разработки](./README.md).

## Защищённое хранение секретов

Защищённое хранение секретов интеграций принадлежит `secrets.rs`: общий
DPAPI-файл `secrets.dat`, версионированные разделы и атомарное обновление.
Twitch использует типизированный адаптер `twitch::credentials`; остальные
интеграции переводятся отдельно. Контракт и ограничения описаны в
[DECISION-025](../decisions/025-shared-secret-store.md).

## Области выполнения горячих клавиш

Горячие клавиши разделены на три независимые области. Глобальные сочетания
регистрируются backend через Windows API. Локальная команда главного окна
обрабатывается внутри окна и может применять политику возврата фокуса.
Сочетания редактора хранятся отдельно в `hotkeys.editor` и обрабатываются
CodeMirror/контейнером вкладок по `KeyboardEvent.code`; они никогда не проходят
через регистрацию global shortcut. Поэтому remap editor binding не приводит к
unregister/reregister системных сочетаний и не действует за пределами редактора.
