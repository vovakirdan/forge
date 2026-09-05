# Forge: Epic Index

Эти документы раскладывают ../IMPLEMENTATION_PLAN.md по planning-иерархии:
Milestone → Epic → Task. Epic не является runtime-сущностью Forge и не создаёт
Task на доске.

| Milestone | Epic | Будущие Task | Exit gate |
|---|---|---|---|
| M0 | M0.1 Workspace и test topology | 01–02 | repeatable local environment |
| M0 | M0.2 Domain и command model | 03–05 | pure rules и mutation boundary |
| M0 | M0.3 Durable scheduler и simulator | 06–10 | fake Pipeline проходит end-to-end |
| M1 | M1.1 Supervisor, sandbox и failure boundary | 11–16 | isolated observed execution |
| M1 | M1.2 Credential-safe first provider lanes | 17–19 | первый real provider без утечки secrets |
| M2 | M2.1 Provider catalog | 20–23 | native CLI contract-tested |
| M2 | M2.2 Git delivery, verification и review | 24–25 | delivery Task проходит engineering loop |
| M2 | M2.3 Project management и resolver queues | 26 | named management без власти Employee |
| M3 | M3.1 Derived memory и retrieval | 27 | bounded derived knowledge |
| M4 | M4.1 Installer и operator acceptance | 28–29 | one-command local MVP proof |

Task-файлы будут созданы только после отдельного перехода к task breakdown.
