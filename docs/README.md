# TTSBard documentation / Документация TTSBard

## User documentation / Пользовательская документация

**[English](en/README.md) | [На русском](ru/README.md)**

<a id="начать-пользоваться"></a>
<a id="подключить-другие-программы"></a>

| Section / Раздел | English | Русский |
|---|---|---|
| Start / Начало | [First launch](en/user/guide.md) | [Первый запуск](ru/user/guide.md) |
| Guides / Руководства | [User guides](en/user/README.md) | [Пользовательские инструкции](ru/user/README.md) |
| Network audio / Звук по сети | [VBAN](en/user/vban.md) | [VBAN](ru/user/vban.md) |
| Connections / Подключения | [Integrations](en/integrations/README.md) | [Интеграции](ru/integrations/README.md) |
| Help / Помощь | [FAQ](en/faq.md) | [Частые вопросы](ru/faq.md) |
| Overview / Обзор | [Product overview](en/ttsbard-overview.md) · [PDF](en/ttsbard-overview.pdf) | [Обзор продукта](ru/ttsbard-overview.md) · [PDF](ru/ttsbard-overview.pdf) |

User guides, FAQ and product overviews (Markdown/PDF) are maintained in `en/`
and `ru/`. Old Markdown pages under `user/` and `integrations/` retain navigation
for existing links; the shared LunaTranslator script stays at its original path.

Актуальные пользовательские инструкции, FAQ и обзоры (Markdown/PDF) находятся
в `en/` и `ru/`. Старые Markdown-страницы в `user/` и `integrations/` содержат
переходы для прежних ссылок; общий скрипт LunaTranslator остаётся на прежнем месте.

## Разработка и история на русском

Документация разработки, исследования, roadmap и история изменений остаются
на русском. Для обычного использования приложения они не требуются.

Development documentation, research, roadmap and changelog remain in Russian.
They are not required for ordinary use.

### Разрабатывать

- [Документация разработки](./development/README.md) — сборка, проверки и workflow.
- [Архитектура](./development/architecture.md) — владельцы состояния и потоки данных.
- [Решения](./decisions/README.md) — принятые варианты и их причины.

### Планы и история

- [История изменений](../CHANGELOG.md) — изменения по версиям, только на русском.
- [Roadmap](./roadmap/README.md) — направления и их lifecycle-статусы.
- [Задачи реализации](./tasks/README.md) — незавершённые долговечные задания.
- [Исследования](./research/README.md) — результаты, открытые вопросы и исторические материалы.

Временные prompts, ревью и логи остаются в `.work/ai/` и не коммитятся.
Завершённая task удаляется после переноса результата в профильный документ.
Правила выбора документа — в [AI-workflow](./development/ai-workflow.md), история
реорганизации — в [ROADMAP-046](./roadmap/completed/046-documentation-structure-migration.md).
