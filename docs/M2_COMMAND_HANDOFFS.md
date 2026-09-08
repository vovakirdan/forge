# M2 — handoff после человеческого решения

Успешный `submit_external_stage_outcome` создаёт новый `TaskHandoff` от
фактически завершённой Human/External стадии к следующей стадии той же Task.
Предыдущая передача от Employee или System executor сохраняется в истории,
но не подставляется вместо нового решения.

Producer — `Command { command_id }`, а не вымышленный Run или Integration.
Handoff содержит реального actor команды, outcome, исходную и целевую стадии,
ссылки на приложенные outcome artifacts и Task work surface, если он есть.
Артефакты остаются submitted: handoff не превращает их в проверенную истину.
Терминальный переход также получает запись; исчерпание лимита посещений без
применённого outcome не получает фиктивного успешного handoff.

Task, очередь, artifacts, events/outbox, receipt и handoff фиксируются одной
транзакцией. Receipt создаётся до handoff внутри этой транзакции: составной FK
и проверка его Project, Task resource, имени команды и actor связывают запись
с реально принятой командой. Одна команда создаёт не более одного handoff.
Replay возвращает прежний receipt без повторной записи. Историю command handoff
нельзя изменить или удалить; связанный receipt также необходимо сохранять.
Его actor, имя команды, fingerprint и содержимое receipt защищены от UPDATE:
ссылка на неизменяемый handoff не может начать описывать другую команду.

Следующий Run получает последний handoff с правильной целевой стадией, не ожидая
summarizer. Строгая проверка scope в `ContextSnapshot` остаётся включённой.

`manual_handoff` command conformance выполняется на memory и PostgreSQL:
атомарные отказы, повтор команды, provenance, artifacts и отказ чужой Task.
PostgreSQL дополнительно проверяет запрет UPDATE/DELETE. Сценарий
`m2_hook_runs::history` проходит work → hook → human → work с новым Git candidate
и проверяет, что прежний hook pass не удовлетворяет новому candidate.
