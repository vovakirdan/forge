# FRONTEND-020 — Список Employee в Team

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-019

## Результат

Owner открывает Team в выбранном Project и видит read-only список Employee из
Core. Карточка содержит только `id`, `name`, `role`, `state`, `revision` и
`max_concurrent_runs`. `state` означает право получить новую работу;
операционную доступность список не утверждает. Hire, профиль, настройки и
provider details остаются следующими задачами.

## Контракт

- `GET /v1/projects/{project_id}/employees?limit=20&cursor=…` проверяет
  существование Project, сортирует по имени и ID, использует общий list cursor.
- Gateway разрешает только scoped GET после owner session/Host/Origin checks,
  ограничивает query и тело ответа 64 KiB. Подпуть Employee закрыт.
- При смене Project/session/вкладки старый read отменяется и cache очищается.
  Ошибки, пустая страница, refresh и stale cursor показаны явно.

## Приёмка

Проверить пустой и полный Project, больше 20 Employee, стабильный порядок,
недействительный cursor, разделение Projects, enabled/disabled/retired,
security boundary, клавиатуру, узкий экран и отсутствие fake availability.
Выполнить Rust/frontend tests, lint/typecheck/build и `just ui-test-live`.
Keyless fixture не служит provider proof.
В текущей canonical схеме имена Employee уникальны внутри Project, поэтому
равные имена нельзя создать штатным API; вторичный порядок по ID закреплён в SQL.

## Evidence

- `just ui-test-live`: 150/150 browser tests PASS, в том числе real Core
  roster с 22 Employee, двумя страницами, пустым Project, тремя состояниями,
  scope switch и cursor recovery. Core egress 2/2 и sandbox 1/1 PASS.
- Скан секретов: 500 issued values в live suite и 504 в ожидаемом failure
  probe, утечек нет. Внешний provider/стенд не проверялись.
- `cargo test -p forge-core --lib --locked`: 81 PASS; `cargo test -p forge-ui
  --lib --locked`: 84 PASS. Frontend contracts/live unit, typecheck, lint
  (0 ошибок, 10 прежних предупреждений), live build, Rust clippy/fmt,
  OpenAPI YAML parse и `git diff --check` PASS.
