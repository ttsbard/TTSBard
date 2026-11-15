# Передача голоса TTSBard со второго ПК по локальной сети

Дата исследования — 2026-11-15. Проверены TTSBard `a4a6acd0`, ShadowBridge v1.0.1 `437e07c` и документация VB-Audio. Статус — маршрут изучен по исходникам и документации, испытание на двух ПК предстоит.

## 1. Потребность

Стример набирает фразы в TTSBard на игровом ПК и использует озвучку как голос в чате игры. TTSBard уже выводит звук в `CABLE Input`, игра читает `CABLE Output` как микрофон. Картинка и звук игры приходят в OBS на стрим-ПК через карту захвата. Наушники подключены к FIFINE SC3 на стрим-ПК.

Та же фраза должна одновременно поступать в голосовой чат игры и по LAN на стрим-ПК. Там она нужна как отдельный источник с собственным ползунком в OBS, как микрофон Discord и, при необходимости, как вход липсинка VTube Studio. При звуке одной игры источник Барда должен молчать.

## 2. Первый вариант — готовый ShadowBridge

Сначала нашли ShadowBridge, который умеет захватывать звук выбранного устройства воспроизведения, передавать его по UDP и воспроизводить на выбранном устройстве другого ПК. В этом варианте работают два экземпляра ShadowBridge, TTSBard продолжает использовать существующий виртуальный микрофон. [Исходники ShadowBridge](https://github.com/shadowsight00/ShadowBridge/tree/437e07ce14b3e6a4f62b07193cb831efb8380a27)

- **Игровой ПК.** Игровой ShadowBridge в режиме `OUT` открывает `CABLE Input` через WASAPI loopback в совместном режиме. Игра читает `CABLE Output`. Производитель VB-CABLE документирует захват `CABLE Input` через опцию **Enable Loopback Streaming**. [Захват OUT](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/capture.rs#L57-L120) · [Руководство VB-CABLE, стр. 13](https://vb-audio.com/Cable/VBCABLE_ReferenceManual.pdf#page=13)
- **Стрим-ПК.** ShadowBridge в режиме `IN` открывает локальный `CABLE Input` как устройство воспроизведения через CPAL. На обоих ПК у канала должен быть один UDP-порт, например `5500`. Для теста стоит выставить 48 кГц в ShadowBridge и в свойствах устройств `CABLE Input`. Захват берёт фактическую частоту устройства, а приёмник воспроизводит с заданной частотой. [Приём IN](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/mod.rs#L222-L294) · [Формат захвата](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/capture.rs#L88-L105)

Ограничения проверенной версии повлияли на выбор долгосрочного маршрута. Если выбранное устройство пропадает, `OUT` переключается на системный выход, поэтому вместо Барда может захватить другой звук. Во время теста нужно проверить фактическое имя устройства и IP назначения в журнале. Режим `APP` тоже захватывает весь микс устройства, через которое играет выбранный процесс. Сетевые команды запуска и остановки принимаются без аутентификации. [Подстановка устройства](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/capture.rs#L68-L86) · [Захват APP](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/capture.rs#L424-L441) · [Управление по UDP](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/net/control.rs#L5-L29)

## 3. Прямая отправка из TTSBard в ShadowBridge

Следующая идея — взять готовый PCM в TTSBard и отправлять его напрямую с игрового ПК. ShadowBridge тогда нужен только на стрим-ПК. Его `IN` принимает сырые стереосэмплы `float32 little-endian` по UDP без заголовка. Стороны заранее согласуют частоту и порт. Автообнаружение, синхронный запуск и WebSocket Stream Deck для такого отправителя не нужны, если приёмник запущен вручную. [Отправка ShadowBridge](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/mod.rs#L319-L369) · [Декодирование пакетов](https://github.com/shadowsight00/ShadowBridge/blob/437e07ce14b3e6a4f62b07193cb831efb8380a27/src-tauri/src/audio/mod.rs#L414-L431)

Это короткий путь к прототипу при уже настроенном ShadowBridge. При этом сетевой выход Барда пришлось бы подстроить под формат ShadowBridge. Сырые UDP-пакеты сами по себе не описывают звук, а приёмники VBAN ожидают свой заголовок и формат потока.

## 4. Выбор совместимого протокола — VBAN

Для постоянной функции TTSBard предпочтителен **VBAN Audio**. Это опубликованный формат аудио поверх UDP с именем потока, частотой, числом каналов, форматом сэмплов и счётчиком кадров. Один отправитель сможет работать с несколькими совместимыми приёмниками. Именно возможность сменить приёмник даёт гибкость в этом сценарии. [Спецификация VBAN](https://vb-audio.com/Voicemeeter/VBANProtocol_Specifications.pdf)

В проверенной ревизии TTSBard встроенный VBAN не обнаружен. Очередь воспроизведения уже получает готовый `AudioPcm`. При реализации сетевой выход можно подключить отдельной веткой, которая отправляет аудиоблоки в темпе воспроизведения и продолжает работу независимо от локального вывода в игровой микрофон. При обрыве сети локальная озвучка должна продолжаться. [Аудиовыходы TTSBard](../ru/user/audio-devices.md) · [Очередь воспроизведения PCM](../../src-tauri/src/playback.rs#L875)

### Готовые программы Windows

| Программа | Роль в схеме |
|---|---|
| **VBAN Receptor** | Один поток на выбранный выход **A1**. Для базового маршрута подходит местный `CABLE Input`. [VB-Audio](https://vb-audio.com/Voicemeeter/vban.htm) · [Настройка с виртуальным кабелем](https://github.com/LizardByte/support-bot-commands/wiki/vban) |
| **VBAN Talkie** | Принимает и отправляет VBAN. Подходит как временный отправитель на игровом ПК для испытания связки с Receptor. [VB-Audio](https://vb-audio.com/Voicemeeter/vban.htm) |
| **Voicemeeter / Banana / Potato** | Принимает VBAN и разводит сигнал на виртуальные выходы. Вариант для нескольких потоков и отдельных миксов. [Voicemeeter](https://vb-audio.com/Voicemeeter/) |
| **VB-Audio Matrix** | Маршрутизирует входящие VBAN-потоки в устройства Windows и виртуальные выходы. [VB-Audio Matrix](https://vb-audio.com/Matrix/) |
| **OBS Studio + `obs-vban`** | Принимает VBAN как отдельный источник OBS. Для Discord и микрофонного липсинка VTube Studio отдельно нужен маршрут в локальное устройство записи. [Плагин](https://github.com/norihiro/obs-vban) · [Сборки Windows](https://github.com/norihiro/obs-vban/releases) |

Для одного потока с общим входом OBS, Discord и VTube Studio достаточно Receptor и VB-CABLE, если эта связка открывает устройства на стрим-ПК. Разработка собственного приёмника оправдана при отдельном требовании, например шифровании и аутентификации аудиотранспорта.

## 5. Схема для проверки на двух ПК

```text
Игровой ПК
TTSBard → местный CABLE Input → местный CABLE Output → микрофон игры
                                   └→ VBAN Talkie → LAN
                                                      ↓
Стрим-ПК                     VBAN Receptor (A1) → местный CABLE Input
                                                      ↓
                                             местный CABLE Output
                                         ┌───────────┼─────────────┐
                                     OBS «Бард»   Discord      VTube Studio

Звук игры → карта захвата → отдельный источник OBS
```

1. На игровом ПК оставить существующий выход TTSBard и микрофон игры. В Talkie выбрать вход `CABLE Output` и отправку на LAN IP стрим-ПК. Задать имя потока, например `TTSBard`, и согласовать порт с Receptor. Стандартный порт VBAN — UDP `6980`. [Утилиты VB-Audio](https://vb-audio.com/Voicemeeter/vban.htm) · [Настройка потока](https://github.com/LizardByte/support-bot-commands/wiki/vban) · [Порт VBAN](https://vb-audio.com/Voicemeeter/VoicemeeterBanana_UserManual.pdf)
2. На стрим-ПК выбрать этот входящий поток в Receptor и выставить **A1 → `CABLE Input`**. На этом этапе проверить, что Receptor видит и открывает местный кабель.
3. В OBS добавить отдельный **Захват входного аудиопотока → `CABLE Output`** с именем `Бард`. В Discord выбрать **Вход → `CABLE Output`** и **Выход → FIFINE SC3**. При необходимости выбрать тот же вход в настройках микрофонного липсинка VTube Studio. Ползунок источника в OBS регулирует только микс OBS. [OBS](https://obsproject.com/kb/audio-sources/) · [Discord](https://support.discord.com/hc/en-us/articles/33030151293079-Discord-Voice-Video-Streaming-Guide) · [VTube Studio](https://github.com/DenchiSoft/VTubeStudio/wiki/Lipsync)
4. После паузы отправить одну фразу «Проверка Барда: один, два, три» при открытом голосовом чате игры. Фразу должно быть слышно в игре и Discord, индикатор источника `Бард` в OBS должен отреагировать. Если включён липсинк, проверить реакцию модели. На локальной записи OBS проверить начало, конец и отсутствие дубля фразы.
5. Дать игре звучать отдельно. Индикатор карты захвата должен реагировать, источник `Бард` должен молчать. Это проверит, что игра, системный звук и мониторинг не попали в виртуальный кабель.

После успешного теста встроенный отправитель VBAN заменит Talkie. Подтверждения с двух ПК нужны для одновременного чтения кабеля игрой и Talkie, открытия `CABLE Input` в Receptor, параллельного чтения `CABLE Output` приложениями на стрим-ПК и оценки задержки. Изменения в коде в рамках исследования не выполнялись.
