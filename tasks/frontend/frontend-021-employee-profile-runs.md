# FRONTEND-021 — Профиль Employee и история Runs

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-020

## Результат

Owner открывает read-only профиль Employee из Team и видит его canonical
identity, role, scheduling state, capacity, stage eligibility, revision и даты.
Отдельная история показывает retained Runs этого Employee, максимум 20 на
страницу, с переходом к уже существующей карточке Run. Ни один из этих фактов
не объявляется текущей доступностью Employee.

## Контракт

- Core `GET /v1/projects/{project_id}/employees/{employee_id}` выдаёт только
  safe profile; чужой Project/Employee отвечает 404.
- Core `GET /v1/projects/{project_id}/employees/{employee_id}/runs?limit=20&cursor=…`
  использует существующий safe `RunView`, сортировку `created_at DESC, id DESC`
  и общий list cursor. Hook/SystemJob без Employee не входят в историю.
- Gateway разрешает только эти два exact GET после owner session/Host/Origin
  проверок, с 1 MiB для профиля и 64 KiB для страницы Runs. Ошибка истории не
  скрывает профиль. Смена Employee, Project, вкладки и сессии очищает старые
  данные и отменяет устаревшие запросы.

## Приёмка

Проверить safe поля, пустую историю, 23 реальных Runs в двух страницах,
порядок и scope, stale cursor, 404, запрещённые методы/query, refresh/error,
фокус клавиатуры и экран 375 px. Выполнить Rust/frontend tests, typecheck,
lint/build, полный `just ui-test-live` и `git diff --check`.

## Границы

Onboarding, hire, amend/enable/disable/retire, provider credentials и
операционная доступность остаются отдельными задачами. Keyless fixture не
является проверкой реального provider.

## Evidence

- `just ui-test-live`: 153/153 browser tests PASS. Новый real Core сценарий
  проверил safe профиль, 23 Runs в двух страницах, порядок, Project isolation,
  stale cursor, пустую историю, клавиатуру и 375 px. Egress 2/2 и sandbox 1/1
  PASS.
- Скан секретов: 508 issued values в live suite и 512 в ожидаемом failure
  probe, утечек нет. Внешний provider/стенд не проверялись.
- `cargo test -p forge-core --lib --locked`: 81 PASS; `cargo test -p forge-ui
  --lib --locked`: 86 PASS. Frontend contracts/live unit, typecheck, lint
  (0 ошибок, 10 прежних предупреждений), live build, Rust clippy/fmt,
  OpenAPI YAML parse и `git diff --check` PASS.
