-- ADR-0098, stage 3. The plugin protocol is version 2: it names no assignment type.
--
-- `plugin_installs.api` is the protocol the installed manifest spoke. It was pinned to '1', so the
-- column could say nothing else; it now takes any number, and the install and the upgrade write
-- the version the core speaks (`plugins::API_VERSION`) instead of a literal. A plugin installed
-- under protocol 1 keeps its '1' until `plugin upgrade` reads a manifest of protocol 2.
ALTER TABLE plugins.plugin_installs DROP CONSTRAINT plugin_installs_api_check;
ALTER TABLE plugins.plugin_installs
    ADD CONSTRAINT plugin_installs_api_check CHECK (api ~ '^[0-9]{1,3}$');

-- The scope `quizzes:read` is `attempts:read` and the event `quiz.attempt_submitted` is
-- `attempt.autograded`. The scope opened the attempts and the questions of a quiz; the questions
-- now come with the assignment under `assignments:read`, so the new scope grants no more than the
-- old one did. What is owed or recorded under the old event name is renamed with it, so nothing
-- in the queues is left that the core can no longer name.
UPDATE plugins.plugin_installs
SET requested_scopes = array_replace(requested_scopes, 'quizzes:read', 'attempts:read'),
    scopes           = array_replace(scopes, 'quizzes:read', 'attempts:read'),
    subscribes       = array_replace(subscribes, 'quiz.attempt_submitted', 'attempt.autograded')
WHERE 'quizzes:read' = ANY (requested_scopes)
   OR 'quizzes:read' = ANY (scopes)
   OR 'quiz.attempt_submitted' = ANY (subscribes);

UPDATE plugins.deliveries
SET event_type = 'attempt.autograded'
WHERE event_type = 'quiz.attempt_submitted';

UPDATE events.outbox
SET type = 'attempt.autograded'
WHERE type = 'quiz.attempt_submitted';
