# FRONTEND-024 — Управление состоянием Employee

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-023

## Результат

Owner включает, отключает и окончательно выводит Employee из работы через
canonical `enable_employee`, `disable_employee` и `retire_employee`. Профиль
показывает только допустимые действия для текущего scheduling state. Перед
командой UI показывает подтверждение и принимает необязательную причину.
Отключение запрещает новые назначения, но не останавливает активные Runs;
retire сохраняет историю Runs.

## Контракт

- Core принимает необязательный `reason` для трёх lifecycle commands и сохраняет
  его как `audit_reason` в Event. Если причина передана, она непустая после
  trim, без NUL и не длиннее 10 000 символов. Для `amend_employee` это поле
  запрещено.
- Gateway пропускает только exact owner routes с Origin/Host guard,
  проверяет project/employee identity, обе revisions и связанный receipt.
- Client получает свежие Project и Employee перед выбором действия. При
  неизвестном исходе повторяет те же bytes и idempotency key. После refusal
  обновляет оба объекта; после receipt перечитывает canonical состояние.
  Ошибка readback оставляет receipt и позволяет повторить только чтение.
- Кэш профиля, Project и Team обновляется в пределах текущего Project и
  сессии. Покидание экрана при отправке или неизвестном исходе предупреждает,
  что уход не отменяет команду.

## Приёмка

Проверить на real Core enable/disable/retire и отсутствие причины, сохранение
причины в audit Event, exact replay, failed readback, stale revisions,
чужой Project, смену Project/сессии, клавиатурный фокус и 375 px. Пройти Rust
и frontend unit tests, typecheck, lint/build, полный `just ui-test-live` и
`git diff --check`.

## Границы

Scheduling state не обозначает живую доступность Employee или физическую
остановку Run. Runtime/provider credentials и onboarding остаются отдельными
задачами. Локальный keyless Core не доказывает работу внешнего provider.

## Evidence

- `just ui-test-live`: 170/170 browser tests PASS, включая шесть новых real
  Core сценариев lifecycle, replay, readback, conflict и поздних ответов.
  Secret scan PASS для 574 issued values; штатная проверка намеренной ошибки
  также прошла (578 values, без утечек).
- `just check`: Rust fmt и clippy workspace PASS. `forge-ui` lib: 95/95 PASS;
  application validation: 1/1 PASS; Employee conformance на memory и
  PostgreSQL: 1/1 каждый. Frontend live unit: 92/92 PASS, typecheck,
  lint (0 ошибок, 10 прежних предупреждений), live build и `git diff --check`
  PASS.
