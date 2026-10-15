# 0011. Submissions: drafts, deadlines, attempts

Status: Accepted

## Context
Students save work and finish it later. Teachers differ in how they treat deadlines and resubmissions.

## Decision
- `submissions.status = draft | submitted`. At most one draft per (assignment, student). Submitted rows are immutable.
- Staff never see drafts.
- Per-assignment settings, chosen by the teacher:
  - `on_deadline = keep_draft | auto_submit` — what happens to a draft at `due_at`;
  - `accept_late` + optional `late_until` — whether late submissions are allowed, and until when; late ones get `is_late = true`;
  - `max_attempts` — `null` = unlimited, `1` = no resubmission.
- Every submit creates a new attempt. Staff grade the latest submitted attempt, and earlier attempts stay visible.
- Auto-submit runs in the worker at `due_at`.
- Mobile clients send an `Idempotency-Key` on submit (ADR-0012).

## Consequences
- The deadline behavior is explicit per assignment, so there are no hidden defaults.
- Autosave of drafts is frequent writes. Fine for Postgres, but drafts emit no events.
