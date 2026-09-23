# FRONTEND-025 — Live-каркас Control Room

**Статус:** done
**Epic:** UI0.2 / UI0.3
**Приоритет:** P0
**Зависимости:** FRONTEND-024

## Результат

Защищённый live entry использует компоновку Control Room: бренд, боковую
навигацию, верхнюю полосу и адаптивную область содержимого. Навигация открывает
только четыре подключённых к Core раздела — Tasks, Team, Runs и Pipeline
versions. Project selector, session и существующие действия продолжают работать
через прежние Core-контракты. Mock-приложение остаётся отдельным demo entry.

## Границы

Sidebar из demo нельзя подключать напрямую: он читает mock services и выводит
выдуманные показатели ресурсов. Live-каркас использует его дизайн-токены и
компоновку, но не импортирует demo services, fake health или неготовые разделы.
Добавление новых разделов, route URLs и Core API остаётся последующими задачами.

## Приёмка и evidence

- Навигация недоступна до выбора Project, переключает только реальные разделы,
  сохраняет leave guard и очищает старые scoped reads.
- `just ui-test-live`: 171/171 PASS, включая новый сценарий на 375 px; secret scan
  PASS для 576 issued values и штатной проверки намеренной ошибки (580 values).
- Frontend typecheck, lint (0 ошибок, 10 прежних предупреждений), live build,
  Prettier для изменённых файлов и `git diff --check` PASS.
