# Epic EXT3.2 — Code intelligence

**Milestone:** EXT3 — Reusable context integrations
**Статус:** Deferred extension; design gate
**Источник:** [UI implementation plan](../UI_IMPLEMENTATION_PLAN.md), [backend alignment](../UI_BACKEND_ALIGNMENT.md)

## Цель

Определить границу optional code-intelligence integration для навигации по репозиторию и evidence-backed context без собственного RAG, graph engine или truth.

## Базовая граница

- M3 хранит canonical project knowledge и derived projections, но не содержит
  product integration code-intelligence provider-а.
- Open-source node graph и session-scoped codebase-memory tooling можно
  переиспользовать как adapter dependency, не как Forge product API.
- Task artifacts, Git/TaskWorkSurface evidence и canonical events остаются
  первичными фактами; code intelligence даёт только contextual discovery.

## Предлагаемое расширение

- Выбрать replaceable external/OSS adapter с явным source snapshot, scope,
  evidence links и bounded query contract.
- Показать result как advisory projection с source revision, freshness и
  availability, а не как semantic truth или acceptance evidence.
- Согласовать permissions, retention, failure/degraded behavior и изоляцию с
  Project, TaskWorkSurface, sandbox и remote/local installation boundary.
- Сохранить Core/Pipeline/Manager write authority независимой от результатов
  graph/search query.

## Не в границах

- Собственная graph database, custom RAG/indexing engine или duplicate исходного кода.
- Автоматическое создание/изменение Task, code write, stage transition или
  решение Manager на основании одного graph result.
- Mandatory indexing service, cross-project source search или скрытый доступ
  к secrets/неразрешённым files.
- Реализация provider integration, UI graph или миграция knowledge storage сейчас.

## Зависимости

- UI3.1 задаёт contract для code-context/read presentation.
- Existing M3 knowledge and retrieval boundary задаёт, что projection не
  становится canonical truth.
- EXT3.2 не блокирует UI0–UI4 или M4; implementation требует отдельного
  approved design и выбора конкретного OSS adapter.

## Исследовательские решения до task breakdown

- Как identity source/repository/branch/SHA и indexing session соотносятся с
  TaskWorkSurface snapshot и сменой Git revision.
- Какой OSS adapter удовлетворяет node/query needs без разработки Forge graph,
  и как его API/error model изолируется через replaceable boundary.
- Какие queries допустимы, как result ссылается на source evidence и как UI
  показывает stale, incomplete или unavailable knowledge.
- Как ACL, sandbox scope, retention, deletion и remote-host privacy исключают
  cross-project leakage и не дают query читать arbitrary files.
- Как budget/capacity и reindex triggers ограничиваются без влияния на
  execution admission, pipeline transition и summarizer authority.

## Будущая декомпозиция

После design approval возможны adapter evaluation, source-snapshot contract,
security boundary, retrieval/read projection и optional UI exploration.

## Exit gate

Утверждённый design record называет выбранный OSS direction, evidence/freshness
semantics, privacy boundary, degraded mode и explicit non-goals. Только затем
возможны отдельные implementation decisions.

## Проверка

- Сверить result contract с Task artifacts, canonical events и M3 knowledge.
- Проверить representative source snapshots, stale index и unavailable adapter.
- Проверить, что UI3.1 не подаёт graph result как факт или permission grant.

## Риски

Неполный graph легко принять за правду, а index — за разрешение читать host; риск снижают evidence links, source-snapshot labels и scope checks.
