# FRONTEND-022 — Создание Employee в Team

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-021

## Результат

Owner создаёт Employee в выбранном Project через canonical `create_employee`.
Форма требует имя, роль и явный выбор `any` либо конкретных Employee stages
из версий Pipeline. Новый Employee появляется только после command receipt и
чтения профиля из Core. Создание не запускает onboarding и не настраивает
runtime.

## Контракт

- Gateway пропускает только exact `POST /api/commands/create_employee` после
  owner session и Origin/Host проверок. Тело закрыто для лишних полей; ответ
  должен содержать receipt с resource kind `employee`, UUIDv7 и следующей
  revision Project.
- Client хранит точное тело и idempotency key при неизвестном исходе и
  повторяет их только по действию owner. Явный conflict/refusal требует
  обновления Project baseline; поля формы остаются на месте.
- После receipt UI читает Project и Employee, обновляет scoped cache и Team.
  При сбое readback receipt остаётся видимым и доступен повтор чтения.

## Приёмка

Проверить `any` и `only`, выбор Employee stages в существующем Project,
недопустимые stage targets и payload, owner guard, Project revision conflict,
replay того же запроса без второго Employee, readback, смену Project/сессии,
клавиатуру и ширину 375 px. Пройти Rust/frontend tests, typecheck, lint/build,
полный `just ui-test-live` и `git diff --check`.

## Границы

Amend/enable/disable/retire, runtime/provider credentials, operational
availability и onboarding остаются отдельными задачами. Keyless fixture не
является проверкой реального provider.

## Evidence

- `just ui-test-live`: 157/157 browser tests PASS, в том числе четыре новых
  real Core сценария создания, ручного replay после потерянного ответа,
  повторного чтения после сбоя readback и stale Project conflict. Egress 2/2,
  sandbox 1/1 PASS.
- Скан секретов: 522 issued values в live suite и 526 в штатной проверке
  намеренной ошибки; утечек нет. Внешний provider/стенд не проверялись.
- `cargo test -p forge-ui --lib --locked`: 89 PASS. Frontend contracts/live unit,
  typecheck, lint (0 ошибок, 10 прежних предупреждений), live build, Rust
  clippy/fmt и `git diff --check` PASS.
