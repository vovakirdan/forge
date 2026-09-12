# Epic EXT3.1 — Skill packs

**Milestone:** EXT3 — Reusable context integrations
**Статус:** Deferred extension; design gate
**Источник:** [UI implementation plan](../UI_IMPLEMENTATION_PLAN.md), [backend alignment](../UI_BACKEND_ALIGNMENT.md)

## Цель

Определить controlled optional registry для reusable skill packs: версионируемых
описаний инструкций, знаний или tool guidance для явного выбора Employee/Run.

## Базовая граница

- Поле `skills` в SQL/Employee metadata не образует registry, package contract
  или механизм распространения capabilities.
- M3 различает canonical project knowledge и derived/retrieval projections;
  оно не публикует SkillPack как продуктовую сущность.
- Provider profile, tool policy, secrets и Employee permissions имеют отдельные
  границы и не должны следовать из текста навыка.
- Текущий MVP не требует marketplace или общий каталог skills.

## Предлагаемое расширение

- Спроектировать immutable pack identity/version, declared content/source,
  owner, visibility и audit выбора конкретной версии для Run.
- Разделить pack guidance от grant capability: наличие skill никогда само не
  выдаёт credential, shell access, MCP server или Manager authority.
- Определить project-scoped publication, review и withdrawal так, чтобы
  historical Run/Task context сохранял выбранную версию.
- Решить, как approved pack соотносится с canonical knowledge без подмены
  KnowledgeBase registry-ем навыков.

## Не в границах

- Marketplace, package execution, remote code install, provider plugin store
  или обязательный набор skills для MVP.
- Хранение secrets, автоматическое повышение прав и обход tool policy.
- Custom RAG, semantic truth validation или изменение memory authority.
- Реализация registry, distribution API, UI catalog либо migration сейчас.

## Зависимости

- UI1.1 задаёт базовые Employee/configuration contracts.
- UI3.2 задаёт нужные surfaces для optional knowledge/skill presentation.
- EXT3.1 не является prerequisite UI0–UI4 или M4; implementation потребует
  отдельного approved design и security review.

## Исследовательские решения до task breakdown

- Какой минимальный pack format допустим: metadata, text/artifacts, source
  references, license/provenance и integrity/version semantics.
- Как scopes project/team/employee соотносятся с ownership, visibility,
  review, archival и historical reproducibility.
- Где проходит граница между skill, prompt fragment, knowledge document,
  provider configuration и executable tool integration.
- Как attach/detach/update pack влияет на Run snapshot, ContextSnapshot и audit.
- Нужны ли import/export adapters для существующих open-source форматов без
  объявления Forge marketplace или runtime package manager.

## Будущая декомпозиция

После design approval возможны registry contract, secure publication workflow,
runtime snapshot integration, audit/read projections и optional UI.

## Exit gate

Утверждённый design record доказывает versioning, provenance, access boundary и
разделение skill/capability. Он содержит explicit non-marketplace scope и
отдельный security approval перед implementation.

## Проверка

- Сверить pack lifecycle с Employee, ProviderProfile, ToolPolicy и M3 knowledge.
- Проверить historical Run, revoked pack и project без skill registry.
- Проверить UI1.1/UI3.2 assumptions: они не требуют skills для базового flow.

## Риски

Registry может стать supply-chain surface или способом выдать права; capability
grants остаются отдельными policy decisions, а packs — optional.
