# Epic UI1.4 — Work surfaces, Artifacts и provenance

**Milestone:** UI1 — browser interface before M4 installer
**Статус на 23 сентября 2026:** локальные surfaces, snapshots, artifacts и handoff reads реализованы и проверены; raw object body не публикуется в браузере.
**Зависимости:** UI0.1 model contracts, UI0.2 browser/API boundary, UI0.3 UI tooling
и UI1.3 Task board/management.
**Контракты:** ../2026-09-03-task-domain-model.md,
../M2_FILE_SNAPSHOTS.md, ../M2_GIT_SOURCE_POLICY.md,
../M2_GIT_BINDING.md и ../M2_GIT_INTEGRATION.md.

## Цель

Дать человеку безопасный Task detail для work surface, Artifact timeline, source
pin, file snapshot, review и Integration candidate provenance. Экран показывает
проверяемые ссылки и canonical receipts, но не открывает host filesystem,
Supervisor internals или object store как общий файловый браузер.

## В границах

- TaskWorkSurface: Git worktree, filesystem sandbox, external binding или no surface без предположения, что каждая Task — Git;
- Artifact timeline: type/format, producer scope, author, stage, Run/ResolutionAssignment, submission, acceptance и visibility;
- immediate TaskHandoff рядом с Artifacts/Run history, отдельно от eventual summarizer output;
- Git source policy/pin для future writer Runs: policy revision, selected revision, descriptor digest и read-only delivery evidence;
- sealed file snapshot/input: manifest metadata, digest, size, executable bit, state и provenance source Task;
- review result/integration candidate: exact candidate/revision, acceptance/result receipt, candidate отдельно от published result;
- scoped object read/preview/download by Project/Task/Artifact через Core API, с redacted metadata и audit-friendly errors.

## Не в границах

- arbitrary host path picker, shell/terminal, Supervisor mounts или raw object-store bucket/key;
- обязательные Git, MR, commit, review или integration для non-Git Task;
- создание/изменение Git candidate, merge, CAS publish или physical capture:
  это существующие management/runtime flows, не browser file access;
- semantic проверка истинности Artifact, Summary или review текста.

## Базовое состояние и разрыв

M2 описывает Git binding/source policy, snapshots, candidate inspection и integration receipts; Task contract — attached Artifact, acceptance и handoff. Existing frontend делает workspace/base SHA/changed files universal и читает mock artifacts. UI0.2 подтверждает safe views/object routes до UI.

Отсутствующий safe projection/scoped read — backend/UI gap. Новая surface kind, Artifact format или retention semantics — domain вопрос, не arbitrary URL/path field.

## Контракт интерфейса

Artifact `submitted` и `accepted` — разные факты. Acceptance относится к stage contract; summary/link/green badge не доказывают качество или truthfulness.

Non-Git Task полноценна: research report, document, external reference или snapshot без MR/SHA. Git panel появляется только для bound surface и не раскрывает checkout path.

Source pin применяется к future writer Run, не переписывает old RunSpec/candidate/accepted Artifact. Snapshot доступен только after sealed; manifest — relative paths/digest, не import root/runtime directory.

Core проверяет Project/Task/Artifact/visibility у каждого object read. Client не строит URL by object key, не принимает local path и не показывает secret diagnostics. Candidate, prepared/observed integration и publication различаются в copy/provenance.

## Зависимости

- UI1.3 даёт Task identity/pinned version/activity/detail shell; UI0.1–UI0.3 — Artifact DTO, scoped API/event boundary и test tools.
- M2 Git/snapshot остаётся source of truth, но UI1.4 не требует Git runtime для non-Git Project.

## Направления будущей декомпозиции

- surface-aware Task detail и Artifact timeline projections;
- safe artifact metadata/preview/download adapter;
- Git source pin and candidate provenance panel;
- file snapshot/input manifest panel и sealed-state handling;
- review/integration receipt presentation with scope and redaction tests.

## Exit gate

Git/filesystem/external-binding/no-surface fixtures показывают разные данные без MR/SHA placeholder. Timeline разделяет submission/acceptance/handoff/summary. Snapshot до `sealed` не читается; после — manifest без host root.

Candidate не выдаётся за published integration: UI показывает provenance/receipt. Cross-project/invisible read получает safe denial. DOM/network/error не содержат host path, bucket key, secret или Supervisor data.

## Проверки

- browser/API fixtures для Git и трёх non-Git surface variants;
- artifact/acceptance/handoff timeline tests с late summary и rejected artifact;
- scoped object-read negatives: foreign Project, hidden Artifact, arbitrary object key/path, pending snapshot и stale manifest;
- provenance regressions: pinned source, candidate, review result, publication receipt и redaction snapshots.

## Риски

Риски: Artifact screen как путь к host data/secrets; candidate как готовый merge или summary как доказательство; Git, навязанный всем Task. Защита — Core-scoped reads, provenance и surface-specific UI.
