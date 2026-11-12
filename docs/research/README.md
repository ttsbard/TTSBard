# Исследования

Здесь сохраняются выводы и основания выбора. Утверждения внутри относятся
к дате исследования; перед новым решением внешние сведения проверяются заново.

## Результаты, использованные в продукте

| Материал | Итог и актуальная инструкция |
|---|---|
| [Улучшение аудиопотока](./08-audio-stream-enhancement-research.md) | DSP и resampling отражены в ROADMAP-029/030; текущая цепочка — [аудиоэффекты](../user/audio-effects.md). Остальные предложения не считаются реализованными. |
| [RUAccent: benchmark и Rust](./13-ruaccent-benchmark-and-rust-port-feasibility.md) | Runtime и выбор модели реализованы в [ROADMAP-082](../roadmap/completed/082-native-ruaccent-rs-runtime.md) и [083](../roadmap/completed/083-ruaccent-omograph-model-selection.md); [установка](../user/ruaccent.md). |
| [LunaTranslator](./15-lunatranslator-input-server-integration.md) | Входящий HTTP-сервер реализован; [настройка интеграции](../integrations/lunatranslator/README.md). |
| [Встроенный OCR](./16-built-in-screen-ocr-feasibility.md) | Однократный захват реализован в [ROADMAP-088](../roadmap/completed/088-one-shot-screen-ocr.md); [руководство](../user/ocr.md). |
| [Benchmark eSlav](./18-ocr-ppocrv5-eslav-benchmark.md) | Основание выбора пакета на указанном корпусе; не гарантия точности на любом экране. [Установка](../user/ocr.md). |
| [MP3 и OGG у Silero](./17-silero-mp3-vs-ogg-format.md) | Сравнение завершено; текущая настройка формата — в [FAQ](../ru/faq.md). |

## Открытые вопросы и условия возвращения

| Материал | Когда возвращаться |
|---|---|
| [Античит](./03-overlay-and-anticheat-compatibility.md) | При воспроизводимом конфликте с конкретной игрой; гарантии допуска античитом нет. |
| [Exclusive fullscreen](./05-exclusive-fullscreen-overlay-problem.md) | При повторении проблемы с известными ОС и режимом игры. Исходная гипотеза не универсальный диагноз. |
| [Инструменты улучшения речи](./07-speech-enhancement-and-restoration-tools.md) | Если текущая обработка не решает измеримый дефект; сначала проверить кандидата на собственном корпусе. |
| [VTube Studio lip-sync](./11-vtube-studio-lipsync-deep-research.md) | При запросе на артикуляцию. Текущая [интеграция](../integrations/vtube-studio.md) передаёт состояние набора. |
| [Stream Deck](./12-stream-deck-integration-research.md) | При спросе на плагин с обратной связью; сначала оценить существующие горячие клавиши. |
| [ruphon](./14-ruphon-phonemizer-reference.md) | При конкретной регрессии Piper/IPA. ROADMAP-082 завершён; старое указание «при старте этапа 4» не является текущим заданием. |
| [Оверлей-браузер Steam и записи](./19-steam-overlay-browser-and-capture.md) | При воспроизводимой проблеме конкретного recorder'а или изменении Steam-клиента. Закрытие вкладки оверлея автоматически невозможно; матрица записи — по состоянию на 2026-09-18. |
| [Снижение F0 и ВЧ в Pocket TTS](./20-pocket-tts-voice-cloning-pitch-and-spectrum.md) | Зафиксировано модельное поведение (`Mimi` ослабляет `>2` кГц на `3.7–5.4` дБ, `FlowLM` занижает F0 на громких женских референсах и чувствителен к RMS входа) и рекомендации по подготовке референса. |

## Исторический контекст

- [Исходная постановка](./initial-product-concept.md) — ранние идеи, не текущий план.
- [Возможности улучшения TTS](./01-tts-improvement-opportunities-2026-06-28.md) — набор идей; принятые результаты отражаются в связанных roadmap, остальные не имеют обещанного срока.

Локальные benchmark-логи и prompts остаются в `.work/ai/`. Пути другой машины
в старых исследованиях фиксируют происхождение измерений, а не обязательный
доступный артефакт нового клона.
