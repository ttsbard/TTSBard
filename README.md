# TTSBard

Text-to-speech for communication and streaming.
Синтез речи для общения и стриминга.

**[English](#english) | [На русском](#русский)**

## English

TTSBard is a Windows application for communicating through text-to-speech in
everyday life and on streams. A global hotkey opens and focuses the editor.
After sending, keep typing, hide the window or return focus to the previous app.

[Download TTSBard](https://github.com/ttsbard/TTSBard/releases) ·
[First launch](docs/en/user/guide.md#first-launch) ·
[Short PDF overview](docs/en/ttsbard-overview.pdf)

### Quick start

Choose and test your audio device in **Audio → Devices**, then set up a voice.
Follow the [first launch guide](docs/en/user/guide.md#first-launch) through your
first spoken phrase.

1. Press `Ctrl+Shift+F3` to open the editor above other windows.
2. Type a phrase and press `Enter`.
3. TTSBard speaks it and, if configured, sends it to OBS, Twitch and VRChat.

Escape and after-send behavior are configurable under Quick editor. The app
runs in the background: minimizing keeps a taskbar entry by default; tray-only
hiding is optional. Starting it again restores the existing instance.

### Features

| Feature | What it does |
|---|---|
| **Quick editor** | Tabs, phrase history, autocomplete, offline Russian spell checking, quick inserts and delivery routes. |
| **TTS** | Local Piper voices and Pocket TTS voice cloning from a WAV sample, Silero Bot through Telegram, OpenAI, ElevenLabs, Fish Audio, and local HTTP servers compatible with TTS Voice Wizard’s Locally Hosted mode. |
| **Local models** | Install and manage Pocket TTS, OCR and RUAccent packages in **Settings → Models**. |
| **Audio cache** | Caches synthesized speech and replays phrases with the same settings without waiting for synthesis or using additional provider quota. |
| **Audio outputs** | Listen through speakers/headphones while sending speech to a virtual microphone for games and chats. |
| **Audio processing** | Pitch, tempo, volume, DeepFilterNet denoising, EQ, compressor and limiter. |
| **Interface languages** | English and Russian; add your own translation with a language pack. |
| **Screen OCR** | Extract text from a selected screen area for speech or editing. |
| **[LunaTranslator](https://github.com/HIllya51/LunaTranslator) integration** | Receive text from games and other apps through LunaTranslator, using the clipboard, screen capture or game engine hooks. |
| **Russian text** | RUAccent places stress marks and resolves pronunciation ambiguities in Russian words; a separate feature converts numbers to Russian words. |
| **Sound panel** | Assign sound sets to a keyboard grid; open with a hotkey and trigger a sound by key. |
| **OBS** | Display phrases as a stream overlay. |
| **Incoming text** | Receive text from other apps over HTTP for speech; a web form supports input through a browser, Steam Overlay or another device on the local network. |
| **Twitch Chat** | Send phrases to chat through the official Twitch API with device-code login and no bot moderator requirement, or through classic IRC. |
| **VTube Studio** | Indicate typing through model animation driven by parameters, configured actions or a scene item appearing. |
| **VRChat** | Send phrases to the avatar chatbox and show typing state. |
| **Compact and Mono** | Compact editor with essential actions; use the menu to hide tabs or switch to the single-field Mono view. Tab visibility is remembered. |
| **Key interception** | Assign individual keys to actions while blocking their normal action in other apps. |
| **AI text correction** | Experimental typo/punctuation correction and style changes before speech. |

See the [product overview](docs/en/ttsbard-overview.md),
[LunaTranslator setup](docs/en/integrations/lunatranslator/README.md) and
[language packs](docs/en/user/language-packs.md).

### Before you start

- Windows 10/11 is supported.
- For Silero Bot, send `/mp3` to [@silero_voice_bot](https://t.me/silero_voice_bot)
  in Telegram to select MP3 responses.
- For hotkey/window issues over an elevated game, try running TTSBard as
  administrator. See the [FAQ](docs/en/faq.md).
- Select **Settings → General → Language** and restart for an English interface.
  This does not change the speech language.
- **Language limitations:** built-in offline spell checking and number-to-word
  conversion currently support Russian only. RUAccent handles Russian stress
  and pronunciation with Silero and Piper; it does not check spelling.
  The default AI correction prompt also targets Russian; replace it for another
  language. **AI: grammar** uses its own fixed Russian instruction. Digits become
  Russian words before AI/TTS, even with an English voice, when number conversion
  is enabled. Turn it off under **Settings → Editor → Convert numbers to words**
  to pass digits to the speech engine. AI correction may still convert numbers.
  See [language limitations](docs/en/user/guide.md#language-support).
  Available speech languages
  depend on the selected TTS provider and voice; changing the interface language
  does not add support for other languages to these text tools.

### Documentation

- [User guide](docs/en/user/guide.md) · [All user instructions](docs/en/user/README.md)
- [Integrations](docs/en/integrations/README.md)
- [Product overview](docs/en/ttsbard-overview.md) · [PDF](docs/en/ttsbard-overview.pdf)
- [FAQ](docs/en/faq.md) · [Documentation map](docs/en/README.md)
- [Changelog](CHANGELOG.md) and [development documentation](docs/development/README.md) remain in Russian.

### Project history

The idea began with a [YouTube interview](https://www.youtube.com/watch?v=O6StzI8CCx0)
featuring [megubite](https://www.twitch.tv/megubite) and
[alexxa69419](https://www.twitch.tv/alexxa69419). Watching alexxa69419 led to
the community of streamers who communicate through TTS, including
[pirunka_tsd](https://www.twitch.tv/pirunka_tsd),
[weissstille](https://www.twitch.tv/weissstille) and
[safutara_kaino](https://www.twitch.tv/safutara_kaino).

They use TTS Voice Wizard with alexxa69419's
[TTS-Wizard-2-Silero-Telegram-Bot](https://github.com/alexxa69420/TTS-Wizard-2-Silero-Telegram-Bot).
The same author created [randompad](https://github.com/alexxa69420/randompad),
an AI-assisted soundpad triggering a random sound from a set with one hotkey.
pirunka_tsd inspired displaying spoken text in OBS for viewers.

TTSBard grew from wanting a quick editor over a game: open, type, send and
return focus. AI tools helped combine this with providers, cache, a soundpad,
OBS text and Twitch delivery in one background application.

### License and support

TTSBard uses [GPL-3.0-only](LICENSE). It is a personal project maintained
voluntarily in spare time, without guaranteed support response/fix deadlines.
It is provided as is. You choose external services; their data policies and
terms apply. The [developer statement](DEVELOPER_STATEMENT.md) gives the full
position and support boundaries in Russian, as of its stated revision date.

Development has used Gemini, OpenAI Codex (GPT-5.6), Claude with Z.ai GLM
models, and DeepSeek v4 Pro through OpenCode for prepared tasks.


---

## Русский

TTSBard — Windows-приложение для общения с помощью синтеза речи в повседневной
жизни и на стримах. Главное окно вызывается глобальной
горячей клавишей, сразу получает фокус для ввода и после отправки может остаться
открытым, скрыться или вернуть фокус предыдущему приложению.

[Скачать TTSBard](https://github.com/ttsbard/TTSBard/releases) ·
[Первый запуск](docs/ru/user/guide.md#первый-запуск) ·
[Краткий обзор в PDF](docs/ru/ttsbard-overview.pdf)

### Быстрый старт

Выберите и проверьте аудиоустройство в **Аудио → Устройства**, затем настройте
голос. [Руководство первого запуска](docs/ru/user/guide.md#первый-запуск) проведёт
вас до первой озвученной фразы.

1. Нажмите `Ctrl+Shift+F3`, чтобы открыть редактор поверх других окон.
2. Введите фразу и отправьте её клавишей `Enter`.
3. TTSBard озвучит текст через выбранный провайдер и, если это настроено,
   отправит его в OBS, Twitch и чатбокс VRChat.

`Escape` и поведение после отправки настраиваются в режиме быстрого редактора.
Приложение продолжает работать в фоне: по умолчанию при сворачивании оно
остаётся на панели задач, а скрытие только в трей включается отдельно. Повторный
запуск показывает уже работающую копию, а не создаёт вторую.

### Ключевые возможности

| Возможность | Что даёт |
| --- | --- |
| **Быстрый редактор** | Вкладки, история фраз, автодополнение, локальная проверка русской орфографии, быстрые вставки и выбор направлений отправки. |
| **TTS** | Локальные голоса Piper и клонирование голоса по WAV в Pocket TTS, Silero Bot в Telegram, OpenAI, ElevenLabs, Fish Audio и локальные HTTP-серверы, совместимые с режимом Locally Hosted в TTS Voice Wizard. |
| **Локальные модели** | Установка и управление пакетами Pocket TTS, OCR и RUAccent в разделе **Настройки → Модели**. |
| **Аудиокэш** | Сохраняет готовую озвучку: повторяющиеся фразы с теми же настройками воспроизводятся без ожидания синтеза и расхода лимитов TTS-сервиса. |
| **Аудиовыходы** | Одновременное воспроизведение в наушники или динамики и виртуальный микрофон для игр и голосовых чатов. |
| **Обработка звука** | Настройка высоты тона, темпа и громкости; шумоподавление DeepFilterNet, эквалайзер, компрессор и лимитер. |
| **Язык интерфейса** | Русский и английский интерфейс; возможность создать и подключить собственный перевод. |
| **OCR экрана** | Распознавание текста из выделенной области экрана для озвучивания или редактирования. |
| **Интеграция с [LunaTranslator](https://github.com/HIllya51/LunaTranslator)** | Получение текста из игр и других программ через LunaTranslator для озвучивания в TTSBard: буфер обмена, захват экрана и подключение к игровым движкам. |
| **Русский текст** | RUAccent для расстановки ударений и выбора произношения слов вроде «за́мок» и «замо́к»; преобразование чисел в слова. |
| **Звуковая панель (саундпад)** | Свои наборы звуков на сетке клавиш. Панель вызывается горячей клавишей, затем нужный звук запускается нажатием назначенной клавиши. |
| **OBS** | Текстовый оверлей с репликами для трансляции. |
| **Входящий текст** | Приём текста из других приложений по HTTP для озвучивания; веб-форма для ввода через браузер, Steam Overlay или другое устройство в локальной сети. |
| **Twitch Chat** | Отправка реплик в чат в двух режимах: официальный Twitch API (авторизация по коду без требования прав модератора для бота) или классический IRC. |
| **VTube Studio** | Индикация набора текста через анимацию модели с помощью параметров, настроенные действия или появление предмета в сцене. |
| **VRChat** | Отправка реплик в чатбокс аватара и индикатор набора текста. |
| **Компактное окно и «Моно»** | Компактный редактор с основными действиями; через меню можно скрыть панель вкладок или перейти в «Моно» с одним полем ввода. Видимость вкладок запоминается. |
| **Перехват клавиш** | Отдельная клавиша запускает выбранное действие TTSBard; её обычное действие в игре или другой программе блокируется. |
| **AI-коррекция текста** | Экспериментальная функция: исправление опечаток и пунктуации, изменение стиля текста перед озвучиванием. |

Создание и подключение собственного перевода описаны в
[инструкции по локализации](docs/ru/user/language-packs.md).

Встроенный [OCR](docs/ru/user/ocr.md) распознаёт выделенную область экрана.
[Интеграция с LunaTranslator](docs/ru/integrations/lunatranslator/README.md)
добавляет получение текста из буфера обмена, отслеживание области экрана
и работу с игровыми движками.

Подробный рассказ о сценариях и возможностях — в
[обзоре продукта](docs/ru/ttsbard-overview.md).

### Важно

- Для озвучивания через Silero Bot выберите формат MP3: отправьте команду
  `/mp3` боту [@silero_voice_bot](https://t.me/silero_voice_bot) в Telegram.
- Если горячие клавиши или вызов окна не работают в игре, попробуйте запустить
  TTSBard от имени администратора — особенно если сама игра запущена с такими правами.
- Приложение работает на Windows 10 и 11.
- **Языковые ограничения:** встроенная локальная проверка орфографии и
  преобразование чисел в слова сейчас поддерживают только русский язык.
  RUAccent обрабатывает ударения и произношение русского текста для Silero
  и Piper; орфографию он не проверяет. Стандартный промт AI-коррекции также
  рассчитан на русский; для другого языка замените его. **AI: грамматика**
  использует свою фиксированную русскую инструкцию. Цифры преобразуются в
  русские слова до AI/TTS даже при английском голосе, если включено преобразование
  чисел. Отключите **Настройки → Редактор → Преобразовывать числа в слова**, чтобы
  передавать цифры движку речи. AI-коррекция всё ещё может преобразовывать числа.
  См. [языковые ограничения](docs/ru/user/guide.md#поддержка-языков).
  Языки озвучивания зависят от выбранного TTS-провайдера
  и голоса; смена языка интерфейса не добавляет поддержку других языков
  в эти инструменты обработки текста.

### Документация

- [Пользовательское руководство](docs/ru/user/guide.md) — настройка, горячие клавиши,
  провайдеры и интеграции.
- [Язык интерфейса](docs/ru/user/language-packs.md) — выбор языка и подключение
  внешних пакетов перевода.
- [Обзор продукта](docs/ru/ttsbard-overview.md) — возможности и сценарии для
  общения и стриминга.
- [Частые вопросы](docs/ru/faq.md) — типовые проблемы.
- [Карта документации](docs/README.md) — все разделы.
- [Документация разработки](docs/development/README.md) — сборка, проверки и
  процесс разработки.

### История проекта

Всё началось с [интервью на YouTube](https://www.youtube.com/watch?v=O6StzI8CCx0) с участием стримеров [megubite](https://www.twitch.tv/megubite) и [alexxa69419](https://www.twitch.tv/alexxa69419).

Наблюдая за трансляциями [alexxa69419](https://www.twitch.tv/alexxa69419), глубже погрузился в тему TTS и через неё узнал о существовании целого комьюнити стримеров, общающихся с аудиторией исключительно с помощью синтеза речи: [pirunka_tsd](https://www.twitch.tv/pirunka_tsd), [weissstille](https://www.twitch.tv/weissstille), [safutara_kaino](https://www.twitch.tv/safutara_kaino).

Стримеры используют программу **TTS Voice Wizard** в связке с Python-скриптом [TTS-Wizard-2-Silero-Telegram-Bot](https://github.com/alexxa69420/TTS-Wizard-2-Silero-Telegram-Bot) (написанным `alexxa69419`) для интеграции с Telegram-ботом [@silero_voice_bot](https://t.me/silero_voice_bot). Также `alexxa69419` разработала вспомогательную утилиту [randompad](https://github.com/alexxa69420/randompad) — написанный с помощью нейросетей саундпад, который при нажатии одной горячей клавиши воспроизводит случайный звук из заранее заданного набора. А у [pirunka_tsd](https://www.twitch.tv/pirunka_tsd) была подсмотрена идея вывода произносимого текста прямо в OBS Studio для удобства зрителей.

Для общения через ТТС хотелось упростить ввод: вызвать редактор поверх игры горячей клавишей, набрать и отправить реплику, затем вернуть фокус игре. Эта идея стала отправной точкой для TTSBard.

ИИ-инструменты помогли воплотить идею единого фонового приложения для быстрого TTS-общения. В нём объединились редактор по горячей клавише, несколько TTS-провайдеров, аудиокэш, саундпад, вывод текста в OBS и отправка реплик в Twitch Chat. Так появился **TTSBard**.

### Лицензия и поддержка

Позиция и границы поддержки описаны в
[заявлении разработчика](DEVELOPER_STATEMENT.md).

TTSBard распространяется по лицензии [GPL-3.0-only](LICENSE).

> В разработке использовались Gemini, OpenAI Codex (GPT-5.6) и Claude с
> моделями GLM от Z.ai. Для части задач применялся DeepSeek v4 Pro через
> OpenCode по заранее подготовленным заданиям.
