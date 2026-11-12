# Документация разработки

Инструкции по сборке, тестированию, отладке, выпуску версий и процессу
разработки. Этот раздел — нормативный источник инженерного workflow; входные
файлы агентов должны ссылаться сюда, а не дублировать команды.

## Локальная рабочая область

Временные задания AI-агентам, промежуточные ревью, логи и скриншоты хранятся в
`.work/ai/<work-id>/`. Каталог `.work/` игнорируется Git и может отсутствовать в
новом клоне. Перед первой AI-задачей его нужно создать вручную:

```powershell
New-Item -ItemType Directory -Force .work/ai | Out-Null
```

Постоянные планы и решения в `.work/` не хранятся.

## Быстрые проверки

```powershell
npm test
npm run build
./scripts/cargo.ps1 test --manifest-path src-tauri/Cargo.toml
./scripts/cargo.ps1 check --manifest-path src-tauri/Cargo.toml
./scripts/cargo.ps1 clippy --manifest-path src-tauri/Cargo.toml
```

`./scripts/cargo.ps1` автоматически находит `libclang.dll` и передаёт все аргументы
в `cargo`, сохраняя код возврата. Обёртка также применяет `RustBinDir` из
`TTSBARD_RUST_BIN_DIR` или `scripts/build.local.psd1` и поднимает его в начало
`PATH`. При необходимости discovery можно вызвать
напрямую: `./scripts/libclang-bootstrap.ps1`.

Проверки выбираются по риску изменения. `npm run build` уже выполняет
`vue-tsc --noEmit`; Rust-тесты по возможности сначала запускаются точечно.

Для focused Rust proof найдите colocated test по имени инварианты и передайте
module/test-name filter через Windows-обёртку. Например:

```powershell
rg -n "failed.*queue|fn .*failed" src-tauri/src/speech_queue.rs
./scripts/cargo.ps1 test --manifest-path src-tauri/Cargo.toml `
  speech_queue::tests::next_actionable_skips_failed_and_selects_next_queued
```

После focused test выполняется более широкий gate, соответствующий риску
изменения. Полный каталог тестов отдельно не поддерживается: source of truth —
имена colocated tests в профильном модуле.

Структура документации проверяется отдельно:

```powershell
./scripts/check-docs.ps1
```

Проверка валидирует локальные Markdown-ссылки и якоря заголовков, lifecycle-статусы
и отсутствие tracked scratch-артефактов. Она также запускается в GitHub Actions.
Поддерживаются ATX-заголовки (`#`–`######`), повторяющиеся заголовки и явные
HTML-якоря; содержимое fenced code blocks не считается документацией. Проверка
не подтверждает доступность внешних URL и соответствие инструкций runtime.

Изолированные проверки валидатора:

```powershell
./scripts/tests/test-check-docs-anchors.ps1
./scripts/tests/test-check-docs-task-lifecycle.ps1
```

### Third-party notices

Непосредственно включённые и vendored сторонние компоненты описывает
[`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md): русский
Hunspell-словарь, eSpeak NG и Signalsmith Stretch/Linear. Это точечный перечень
прямых assets, а не сгенерированный реестр всех транзитивных Rust/npm
зависимостей.

Данные русского словаря (`ru.aff`/`ru.dic`) встроены в exe на этапе компиляции
через `include_str!`. В репозитории остаются исходные файлы словаря, а в поставке
— его лицензия `third-party/licenses/hunspell-ru-LICENSE.txt`; внешние файлы словаря в bundle не
входят.

Проверка запускается отдельно:

```powershell
./scripts/check-third-party-notices.ps1
```

Она сверяет SHA-256 исходных файлов словаря, наличие notice/license files,
pinned revisions (LibreOffice, piper-rs, eSpeak NG), явный license mapping
`third-party/licenses/hunspell-ru-LICENSE.txt` и сохранение `bundle.licenseFile`. Валидатор
отвергает resource mappings, которые упаковали бы словарь целиком: каталог
`resources/dict`, глоб `resources/dict/*`, отдельные `ru.aff`/`ru.dic` или
родительский каталог `resources`. Проверка не обращается к сети и не валидирует
полный dependency inventory — только перечисленные прямые assets.

### Каталог локальных моделей

Схема, контрольные суммы inline-файлов и пути в
[`catalog/model-catalog.json`](../../catalog/model-catalog.json) проверяются
скриптом без сетевых зависимостей:

```powershell
python scripts/validate-model-catalog.py
python -m unittest discover -s scripts/tests -p "test_*.py" -v
# С проверкой доступности всех внешних URL (HTTP 200):
python scripts/validate-model-catalog.py --check-urls
```

Offline-проверка и unit-тесты запускаются в GitHub Actions на каждом прогоне CI,
а `--check-urls` — дополнительно при изменении `catalog/model-catalog.json` или
самого валидатора.

## Сборка приложения на Windows

```powershell
./scripts/build.ps1 -Mode debug
./scripts/build.ps1 -Mode release
```

Debug-режим по умолчанию создаёт runnable `src-tauri/target/debug/ttsbard.exe`
без инсталляторов, release-режим — приложение и bundles в
`src-tauri/target/release/bundle/`. Это расположение по умолчанию; при
переопределении Cargo target directory (см. «Локальная конфигурация» ниже)
артефакты окажутся в выбранном каталоге. Флаг `-Clean` удаляет build outputs
перед сборкой; применять его следует только при доказанной проблеме кэша.

Для запуска двойным кликом доступны `scripts/build-debug.bat` и
`scripts/build-release.bat`. `scripts/build.ps1` сохранён как UTF-8 с BOM для
совместимости кириллицы с Windows PowerShell 5.1 — при редактировании BOM нужно
сохранить.

### Локальная конфигурация

Разработчик может переопределить Cargo target directory и путь к Rust toolchain
через персональный конфигурационный файл, переменные окружения или параметры
командной строки. Это позволяет разным разработчикам на разных машинах
использовать `build.ps1` без правок.

Приоритет (от высшего к низшему):
1. Явный параметр командной строки (`-CargoTargetDir`, `-RustBinDir`);
2. Переменная окружения (`CARGO_TARGET_DIR`, `TTSBARD_RUST_BIN_DIR`);
3. Файл `scripts/build.local.psd1`;
4. Умолчания (`src-tauri\target`, без переопределения PATH).

Пример конфигурационного файла: `scripts/build.local.example.psd1`. Скопируйте
его в `scripts/build.local.psd1` (игнорируется Git) и отредактируйте:

```powershell
Copy-Item scripts/build.local.example.psd1 scripts/build.local.psd1
# Отредактируйте scripts/build.local.psd1
```

Затем запускайте сборку как обычно:

```powershell
./scripts/build.ps1 -Mode debug
# или с явным параметром:
./scripts/build.ps1 -Mode debug -CargoTargetDir D:\my-target
```

Разрешённые ключи в `.psd1`: `CargoTargetDir` и `RustBinDir` (оба строковые или
`$null`). Ссылки `%NAME%` в путях раскрываются в значения переменных окружения.
Относительные пути разрешаются относительно корня репозитория.

При использовании внешней Cargo target directory (не `src-tauri\target`) перед
первым `-Clean` необходима одна не-clean сборка, которая создаст маркерный файл
`.ttsbard-build-target` внутри target-каталога. Этот маркер защищает от
случайного удаления нецелевой директории. После разрешённой очистки каталог и
маркер создаются заново.

Фактическое расположение артефактов зависит от настроек целевой директории и
выводится скриптом в начале и в конце сборки.

## Документы

### Пользовательская документация EN/RU

Пользовательские материалы находятся в `docs/en/` и `docs/ru/` с одинаковыми
именами и вложенностью. Корневой README содержит оба языка целыми блоками;
`CHANGELOG.md` остаётся только на русском. Документация разработки, research,
roadmap и decisions не переводятся. Корневые инструкции агентов — на английском.

При изменении пользовательского поведения обновляйте обе языковые страницы
и затронутые тексты краткого PDF. Названия элементов интерфейса сверяйте
по одному ключу в `locales/en.json` и `locales/ru.json`, native-строки — также
в `src-tauri/locales/en.json` и коде. Не переводите API-ключи полей, команды
или имена файлов. Язык интерфейса не расширяет языковые возможности моделей.
Если перевод временно отстаёт, укажите на нём дату/версию последней сверки
и конкретные устаревшие разделы; не оставляйте незаметно старую инструкцию.

На каждой странице первая строка переключает язык той же темы. Обычные ссылки
ведут внутри языка. Ссылки на инженерные материалы помечаются «на русском» /
«Russian». Старые Markdown-пути внутри `docs/user/` и `docs/integrations/` содержат только переходы с
сохранёнными якорями; новый текст пишется в языковых файлах. Скрипт
LunaTranslator остаётся единым в `docs/integrations/lunatranslator/selfbuild_tts.py`.

Проверки:

```powershell
./scripts/check-docs.ps1
./scripts/tests/test-check-bilingual-docs.ps1
```

Первая команда включает проверку языковых пар, содержательных страниц,
переключателей, локальных ссылок и якорей. Это структурная проверка;
эквивалентность перевода и правильность инструкции проверяются отдельно.

### Сборка краткого PDF на двух языках

Генератор `scripts/build-overview.py` сохраняет утверждённое оформление
краткого двухстраничного обзора. `scripts/overview-content/en.json` и `ru.json`
хранят его тексты, `scripts/overview_style.py` — общие стили. Это самостоятельный
краткий обзор, не автоматическая копия подробного `ttsbard-overview.md`.

Поддерживаемое окружение: Python 3.10+ и Windows 10/11 с Segoe UI и Consolas.
Для другого окружения предусмотрен `--font-dir` с DejaVu Sans, Bold и Mono;
после смены шрифтов заново проверьте вёрстку. Зависимости:

```powershell
python -m pip install -r scripts/requirements-pdf.txt
python scripts/build-overview.py
# Дополнительно: копии с версией/языком в имени для вложения релиза
python scripts/build-overview.py --release-dir .work/ai/pdf-release
```

Сборка берёт версию из `package.json` и создаёт PDF только в `docs/en/` и
`docs/ru/`; прежняя копия в корне `docs/` больше не создаётся.
Не требуется `.work/` или локальный релизный скрипт для обычной сборки.
Формат A4, две страницы; раздел «Вывод звука» / Audio outputs начинает вторую.
Сначала редактируйте тексты, затем собирайте обе версии и просматривайте все
четыре страницы через PDF-рендерер. Проверяйте переносы, кириллицу, отсутствие
обрезания и кликабельные ссылки. Исторические PDF не перевыпускаются.

- [Карта UI](./ui-map.md) — экраны, вложенные поверхности и владельцы оформления.
- [Проверка и согласование UI](./ui-guidelines.md) — критерии UI-проверки,
  артефакты прогона и статус визуальных ориентиров.

- [AI-assisted development workflow](./ai-workflow.md) — постановка локальных
  задач DeepSeek, запуск OpenCode и независимая проверка результата.
- [AI-ready: принципы](./ai-ready-principles.md) — норматив наблюдаемой стоимости
  безопасного малого изменения для агента без памяти о прошлых сессиях.
- [Шаблон code review](./templates/code-review.md) и
  [шаблон AI-ready review](./templates/ai-ready-review.md) — проверка конкретной
  реализации и ручная оценка удобства следующего безопасного изменения.
- [Ручные тест-кейсы](./test-cases.md) и
  [шаблон тест-кейсов](./templates/test-cases.md) — норматив формирования
  ручной проверки функционала, затронутого roadmap-этапом, поверх зелёных
  автотестов.
- [Архитектура](./architecture.md) — устройство приложения и основные
  инженерные паттерны.
- [Сборка и релиз Windows в GitHub Actions](./github-actions-build.md) —
  устройство CI, теги и нативные зависимости.
- [Локальный ONNX Runtime для Piper](./debug-piper-onnx-runtime.md) — подготовка
  debug-сборки.
- [HTTP-диагностика ElevenLabs](./debug-elevenlabs-logging.md) — безопасное
  debug-only логирование запросов и ответов без API-ключа и текста синтеза.
- [Смешанная CRT в debug-сборке Windows](./windows-debug-crt.md) — диагностика
  и проверка Windows runtime.
