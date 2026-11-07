-- What the Buddy proposes and the user has not yet confirmed (ADR-0070). Nothing here is a change to
-- the school's data: a confirm runs the service function of the core, in the user's own request.
-- `args` and `facts` are JSON in `text` (the workspace does not enable sqlx's `json`, ADR-0014):
-- `args` is what the confirm runs, `facts` is what the card shows (titles and names read from the
-- service, never taken from the model).
CREATE TABLE mod_ai_agent.proposals (
    id         uuid        PRIMARY KEY,
    user_id    uuid        NOT NULL,
    tool       text        NOT NULL,
    args       text        NOT NULL,
    facts      text        NOT NULL,
    status     text        NOT NULL DEFAULT 'pending'
                           CHECK (status IN ('pending', 'done', 'rejected', 'failed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    decided_at timestamptz
);
CREATE INDEX proposals_by_user ON mod_ai_agent.proposals (user_id, id);
CREATE INDEX proposals_by_age ON mod_ai_agent.proposals (created_at);
