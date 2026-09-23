# FRONTEND-023 — Редактирование Employee

**Статус:** done
**Epic:** UI1.1 / UI0.2
**Приоритет:** P0
**Зависимости:** FRONTEND-022

## Результат

Owner меняет имя, роль, вместимость и stage eligibility существующего Employee
через canonical `amend_employee`. Форма отправляет только изменённые поля.
Retired Employee не редактируется; изменение вместимости не останавливает Run.

## Контракт

- Gateway пропускает только exact `POST /api/commands/amend_employee` через owner
  session и Origin/Host guard. Непустой patch закрыт для лишних полей и null;
  receipt должен указывать на того же Employee и следующую Project revision.
- Client хранит точные тело и idempotency key при неизвестном результате.
  Refusal/conflict сохраняет ввод и требует явного обновления Project и Employee
  baselines, затем нового действия owner.
- После receipt UI читает оба canonical объекта, обновляет Team и профиль.
  При сбое readback receipt остаётся видимым; retry повторяет только GET.

## Приёмка

Проверить четыре поля и их комбинацию на real Core, пустой/hostile patch,
чужой Project, неверные stage targets, stale Project/Employee revision,
точный replay, failed readback, смену Project/сессии, клавиатуру и 375 px.
Пройти Rust/frontend tests, typecheck, lint/build, полный `just ui-test-live`
и `git diff --check`.

## Границы

Enable/disable/retire, runtime/provider credentials, operational availability
и onboarding остаются отдельными задачами. Keyless fixture не доказывает
работу внешнего provider.

## Evidence

- `just ui-test-live`: 164/164 browser tests PASS, в том числе семь новых
  real Core сценариев: четыре поля, hostile/scope/conflict, точный replay,
  readback recovery, guard и запоздалые ответы после Project switch/logout.
  Egress 2/2 и sandbox 1/1 PASS.
- Скан секретов: 550 issued values в live suite и 554 в штатной проверке
  намеренной ошибки; утечек нет. Внешний provider/стенд не проверялись.
- `cargo test -p forge-ui --lib --locked`: 92 PASS. Frontend contracts/live
  unit, typecheck, lint (0 ошибок, 10 прежних предупреждений), live build,
  Rust clippy/fmt и `git diff --check` PASS.
