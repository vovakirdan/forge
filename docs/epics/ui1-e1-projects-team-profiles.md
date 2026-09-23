# Epic UI1.1 — Projects, команда и профили Employee

**Milestone:** UI1 — browser interface before M4 installer
**Статус:** `in_progress`; priority read FRONTEND-012, Project selector
FRONTEND-019 и Team read FRONTEND-020/021 завершены как отдельные срезы,
полные зависимости UI0.1/UI0.2/UI0.3 и exit gate сохраняются.
**Зависимости:** UI0.1 model contracts, UI0.2 browser/API boundary, UI0.3 UI tooling.
**Контракты:** ../2026-09-03-employee-domain-model.md,
../2026-09-03-system-manager-domain-model.md, ../2026-09-11-m3-specs.md и
../M2_PROVIDER_PROFILES.md.

## Цель

Дать человеку project-scoped экран Project и Employee, где он видит безопасную
конфигурацию и операционные projection, создаёт или меняет Employee только
именованными командами и понимает состояние onboarding. UI не становится
источником identity, provider session, секретов или scheduler truth.

## В границах

- список Projects, active Project в route/query keys и safe metadata: name, key, description, status и разрешённые config links;
- каталог/profile Employee: identity, role, capabilities, stage eligibility, scheduling lifecycle и отдельная operational projection;
- hire, разрешённый amend, enable, disable и retire через commands с revision, receipt, actor и reason;
- active/previous Runs как история исполнения, а не постоянный владелец Task или один `currentRunId`;
- runtime/provider preference отдельно от Employee identity, без credential material и provider session;
- onboarding gate: status, personal-note/receipt/familiarity metadata, request/retry и audited skip с reason;
- четыре текущих lanes: `codex_cli`, `claude_cli`, `openrouter_api`, `openai_api`; только capability/readiness, без auth/token/key и неявного выбора модели.

## Не в границах

- credential management, provider home, live M2 acceptance и смена runtime уже созданного Run;
- stop/force-stop/continue Run, управление queue или Project gate: это UI2.3;
- глобальные accounts/RBAC, remote control, installer wizard и M4 deployment;
- автоматический onboarding через невыбранную или unconfigured LLM.

## Базовое состояние и разрыв

M0–M3 задают canonical Employee, named commands, receipts, onboarding и four-lane provider profiles. Это не доказывает готовый browser projection: UI0.2 сначала фиксирует read/write API.

Текущий extracted frontend читает mock services, хранит active Project локально и смешивает provider с Employee, одного manager с иерархией и `currentRunId` с историей. Это UI/API разрыв, не разрешение создать browser state machine.

Отсутствующий safe read-model/command — явный backend contract gap, не оптимистичный локальный объект.

## Контракт интерфейса

Project ID входит во все query keys, route parameters и command envelopes;
переключение Project очищает или изолирует старый cache. Любая cross-project
ссылка показывает отказ, а не fallback на первый Project.

Employee lifecycle `enabled`/`disabled`/`retired` показывается отдельно от
`busy`/`idle`/`unreachable` и числа Run. Disable запрещает будущий admission, но
не обещает остановить активный Run; retire сохраняет историю.

Hire и amend передают expected revision и после ответа показывают canonical
receipt или conflict. UI не назначает Employee владельцем Task и не скрывает
несколько Runs одним status badge.

Onboarding request возможен только с явно выбранным, доступным SystemJob/runtime
profile. Если такого профиля нет, UI показывает unavailable и не вызывает LLM.
Skip требует reason, actor и receipt; legacy bypass остаётся отдельным состоянием,
а не ложным completed onboarding.

## Зависимости

- UI0.1 определяет DTO/lifecycle/authority; UI0.2 — authenticated transport, query/error и refresh; UI0.3 — component/testing/tooling baseline.
- M3 onboarding и M2 provider profiles остаются backend contracts, не UI gates.

## Направления будущей декомпозиции

[FRONTEND-012](../../tasks/frontend/frontend-012-project-priorities.md) добавляет
отдельный read схемы приоритетов проекта, без расширения ProjectView. Он нужен
для названий в Task и будущего выбора при создании; настройка схемы, остальные
каталоги, Project list и Team не входят. Отказ каталога не блокирует control read.

[FRONTEND-019](../../tasks/frontend/frontend-019-project-selector.md) добавляет
safe Project list с постраничным выбором в live UI. Это не Employee catalog,
настройка Project или закрытие полного UI1.1 gate.

[FRONTEND-020](../../tasks/frontend/frontend-020-team-roster.md) добавляет
постраничный read-only Employee list в выбранном Project с ролью, scheduling
state и capacity. Operational availability остаётся отдельным read gap.

[FRONTEND-021](../../tasks/frontend/frontend-021-employee-profile-runs.md) добавляет
read-only профиль Employee и его retained Run history. Operational availability,
onboarding и команды управления остаются отдельно.

- Project settings и более полная safe config metadata;
- operational projection доступности Employee отдельно от scheduling state;
- hire/configuration forms и receipt/conflict presentation;
- onboarding status/request/retry/skip и four-lane readiness view;
- browser contract fixtures для unavailable, forbidden и stale-revision ответов.

## Exit gate

В двух Projects switch не показывает чужих Employee, Runs или cache. Hire создаёт Employee только после receipt; reload не зависит от local draft. Disable не выдаётся за stop, retire не удаляет history, provider preference не меняет identity.

Новый Employee ясно показывает onboarding gate. Request без configured runtime не запускает LLM; skip имеет audited reason, четыре lanes видны без secrets/config contents.

## Проверки

- browser/API contract tests для scope, empty/error/conflict и cache switch;
- command tests для hire/amend/enable/disable/retire с receipt и stale revision;
- onboarding fixtures для pending, completed, failed, explicit skip и legacy bypass;
- redaction assertions: DOM/network diagnostics не содержат token, auth path или provider session material.

## Риски

Риски: mock state как управление Core; Employee, слитый с provider/одним Run; unavailable onboarding как скрытый LLM call. В каждом случае UI показывает refusal или receipt, не анимацию успеха.
