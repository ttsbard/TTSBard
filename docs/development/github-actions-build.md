# Сборка Windows в GitHub Actions

## Текущий workflow

Windows-сборка настроена в `.github/workflows/build.yml` через runner
`windows-latest` и target `x86_64-pc-windows-msvc`.

Workflow ищет `libclang.dll` в каталогах LLVM и Visual Studio, задаёт
`LIBCLANG_PATH` и останавливает сборку, если библиотека отсутствует. Состав
образа можно сверить со [списком ПО Windows runner](https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md#installed-software).

## Релизный запуск

Workflow запускается для push числового тега в формате `1.8.1` и вручную через `workflow_dispatch`.
Только запуск по тегу создаёт GitHub Release; ручной запуск собирает
dev-артефакты.

### Подпись обновлений

Для встроенного обновления GitHub Actions подписывает NSIS-установщик
и публикует его вместе с отдельным файлом `*.exe.sig`. В настройках репозитория
задайте переменную Actions `TAURI_UPDATER_PUBLIC_KEY` содержимым файла
`.pub`. Закрытый ключ хранится в секрете `TAURI_SIGNING_PRIVATE_KEY`,
а пароль, если задан, в `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

`scripts/build.ps1 -Mode release -UpdaterArtifacts` передаёт публичный
ключ в Tauri через временный конфиг сборки. Собранный Бард использует
его для проверки подписи скачанного установщика. Если в Actions задан
только один из ключей, сборка выдаёт ошибку. Если оба отсутствуют,
обычная сборка выпускает установщик для ручного обновления.

### Описание релиза и заметки обновления

Релизные заметки берутся из одного источника: секции `CHANGELOG.md` для
тега (`## [1.9.0] - дата`). Workflow извлекает её через
`node scripts/make-update-manifest.cjs --print-notes` и записывает как описание
GitHub Release. Бард читает версию, время публикации, описание и список
файлов через GitHub Releases API. Для установки нужны пара
`TTSBard_<версия>_x64-setup.exe` и `TTSBard_<версия>_x64-setup.exe.sig`.

Релиз создаётся черновиком и публикуется после загрузки файлов. При сборке
без ключей установщик доступен для ручной установки. Со сборок, которые
ожидают `latest.json`, первый переход на эту схему выполняется вручную.
Проверка извлечения заметок на фикстурах
`node --test scripts/make-update-manifest.test.mjs`.

### Ручной smoke подписанного обновления (две версии)

Для проверки перехода между двумя версиями используйте отдельный публичный
тестовый репозиторий с настроенными `TAURI_UPDATER_PUBLIC_KEY` и
`TAURI_SIGNING_PRIVATE_KEY`. В обеих тестовых сборках укажите его URL
в константе `RELEASES_API_URL` в `src-tauri/src/updates/mod.rs`.

1. Собрать и выпустить первую проверяемую версию как релиз `1.8.1`
   через штатный теговый запуск (NSIS-установщик и отдельный `.sig`).
2. Установить `1.8.1` из NSIS-установщика на чистой Windows-машине; убедиться,
   что «Настройки → Общие → Обновления» показывает установленную версию и проверка
   отвечает «Установлена актуальная версия».
3. Выпустить следующий по номеру релиз `1.8.2` тем же
   способом (его секция в `CHANGELOG.md` заполнена — она попадает в
   описание релиза).
4. В установленной `1.8.1` нажать «Проверить сейчас»: карточка должна
   показать `1.8.2`, кнопка «Обновить» появляется после проверки наличия пары файлов.
5. Запустить установку: passive-NSIS обновляет приложение, после
   перезапуска версия — `1.8.2`, сохранённые настройки и вкладки на месте.
6. Повторить проверку на `1.8.2`: обновлений нет.

В смоуке от 18 ноября 2026 года подписанный установщик запустился на Windows.
Переход между разными внутренними версиями и установка сборки из GitHub
Actions ждут проверки. Подробности зафиксированы в [ROADMAP-141](../roadmap/active/141-github-app-updates.md).

Проверки документации, форматирования, Clippy, TypeScript/IPC и тесты выполняет
отдельный workflow `CI` при push в `master` или pull request. Clippy и Rust-тесты
запускаются на целевой платформе `windows-latest` через `scripts/cargo.ps1`;
документация, форматирование и frontend-проверки остаются на Ubuntu. CI также
запускает `scripts/check-third-party-notices.ps1`, который сверяет provenance
непосредственно включённых сторонних assets (словарь, eSpeak NG, Signalsmith) —
он проверяет только перечисленные прямые assets, а не полный реестр
транзитивных зависимостей. Теговый
запуск `Build and Release` ждёт успешный `CI` для того же commit SHA (не более
30 минут) и только после этого начинает release-сборку. Это исключает повторный
прогон тестов и Windows debug-сборку. Ручной запуск не требует предварительного
`CI`.

Перед тегом синхронизируйте версию штатным скриптом, проверьте diff и сборку:

```powershell
node scripts/set-version.cjs 1.9.0
npm run build
./scripts/cargo.ps1 check --manifest-path src-tauri/Cargo.toml --locked
```

Скрипт обновляет `package.json`, `src-tauri/Cargo.toml`,
`src-tauri/tauri.conf.json` и `src/version.ts`. Также синхронизируйте версию
корневого пакета в `package-lock.json` (верхний уровень и `packages[""]`)
и запись пакета `ttsbard` в `src-tauri/Cargo.lock`, не меняя версии зависимостей.
Заполните секцию `## [1.9.0]` в `CHANGELOG.md` и выполните
`./scripts/check-docs.ps1`. До отправки тега проверьте, что команда
`node scripts/make-update-manifest.cjs --print-notes --version 1.9.0 --changelog CHANGELOG.md`
выводит текст этой секции. Полную локальную релизную сборку
запускайте через `./scripts/build.ps1 -Mode release`.

После одобрения и создания коммита версии сначала отправьте ветку. Тег можно
отправить сразу: release workflow дождётся результата `CI` для того же commit
SHA. Если `CI` завершится ошибкой, выпуск не будет собран и опубликован.

Создайте и отправьте тег, указывающий на нужный commit (номер ниже — пример
текущего релиза):

```powershell
git tag 1.9.0
git push origin 1.9.0
```

В CI версия извлекается из имени тега и повторно применяется через
`scripts/set-version.cjs`. Отдельной версии внутри workflow нет. Если требуется
повторить сборку без нового тега, используйте ручной запуск; он не создаёт
release автоматически.

## Кэширование

Кэш npm включён в `actions/setup-node` через `cache: npm`. Rust-кэш настроен
после шага `Setup Rust`:

```yaml
- name: Cache Rust
  uses: Swatinem/rust-cache@v2
  with:
    workspaces: src-tauri
```

Кэшируются Cargo registry, Git-зависимости и `src-tauri/target`. Это особенно
важно для тяжёлых зависимостей Piper и DeepFilterNet (`ort`, `espeak-rs`,
`deep_filter`). Первый запуск создаёт кэш, последующие сборки используют его.

## Проверка нативных инструментов

Перед `npm run tauri build` рекомендуется проверять CMake и `libclang.dll`:

```yaml
- name: Verify native tools
  shell: pwsh
  run: |
    cmake --version

    $paths = @(
      'C:\Program Files\LLVM\bin\libclang.dll',
      'C:\Program Files\Microsoft Visual Studio\*\*\VC\Tools\Llvm\x64\bin\libclang.dll'
    )

    $libclang = $paths |
      ForEach-Object { Get-ChildItem $_ -ErrorAction SilentlyContinue } |
      Select-Object -First 1

    if (-not $libclang) {
      throw 'libclang.dll not found'
    }

    "LIBCLANG_PATH=$($libclang.DirectoryName)" >> $env:GITHUB_ENV
    Write-Host "Using libclang: $($libclang.FullName)"
```

`espeak-rs-sys` использует bindgen и CMake, поэтому без `libclang.dll` и CMake
сборка Windows завершится до создания приложения.

## Фиксация зависимостей

В репозитории должны находиться:

- `Cargo.lock`;
- `package-lock.json`;
- `.github/workflows/build.yml`.

Это обеспечивает воспроизводимое разрешение Rust- и npm-зависимостей и делает
ключи кэша стабильными.
