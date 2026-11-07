//! SQL only. The tables are the module's own, in `mod_ai_agent` (ADR-0066).

use sqlx::PgConnection;
use uuid::Uuid;

use crate::model::{ChatMessage, ChatRole, Proposal};

/// Counts one more message today unless `cap` is reached (`0`: no limit). One statement, so two
/// requests at once cannot both take the last one.
pub async fn take_message(
    conn: &mut PgConnection,
    user_id: Uuid,
    cap: i32,
) -> Result<bool, sqlx::Error> {
    let taken = sqlx::query!(
        "INSERT INTO mod_ai_agent.usage AS u (user_id, day, count)
         VALUES ($1, current_date, 1)
         ON CONFLICT (user_id, day) DO UPDATE SET count = u.count + 1
         WHERE $2 = 0 OR u.count < $2
         RETURNING count",
        user_id,
        cap,
    )
    .fetch_optional(conn)
    .await?;
    Ok(taken.is_some())
}

/// Gives back the message `take_message` counted, when the model could not answer.
pub async fn release_message(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE mod_ai_agent.usage SET count = greatest(count - 1, 0)
         WHERE user_id = $1 AND day = current_date",
        user_id,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The last `limit` messages of the user, oldest first.
pub async fn recent(
    conn: &mut PgConnection,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    sqlx::query_as!(
        ChatMessage,
        r#"SELECT id AS "id!", role AS "role!: ChatRole", content AS "content!",
                  created_at AS "created_at!"
           FROM (SELECT id, role, content, created_at FROM mod_ai_agent.messages
                 WHERE user_id = $1 ORDER BY id DESC LIMIT $2) AS latest
           ORDER BY id"#,
        user_id,
        limit,
    )
    .fetch_all(conn)
    .await
}

pub async fn insert_message(
    conn: &mut PgConnection,
    user_id: Uuid,
    role: ChatRole,
    content: &str,
) -> Result<ChatMessage, sqlx::Error> {
    sqlx::query_as!(
        ChatMessage,
        r#"INSERT INTO mod_ai_agent.messages (id, user_id, role, content)
           VALUES ($1, $2, $3, $4)
           RETURNING id, role AS "role: ChatRole", content, created_at"#,
        Uuid::now_v7(),
        user_id,
        role.as_str(),
        content,
    )
    .fetch_one(conn)
    .await
}

/// Deletes the user's chat and what was proposed in it. Today's count stays, so clearing the chat
/// does not reset the cap.
pub async fn clear(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM mod_ai_agent.messages WHERE user_id = $1",
        user_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM mod_ai_agent.proposals WHERE user_id = $1",
        user_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes messages older than `days` days and the counts of days long past.
pub async fn purge(conn: &mut PgConnection, days: i32) -> Result<u64, sqlx::Error> {
    let messages = sqlx::query!(
        "DELETE FROM mod_ai_agent.messages WHERE created_at < now() - make_interval(days => $1)",
        days,
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    sqlx::query!(
        "DELETE FROM mod_ai_agent.proposals WHERE created_at < now() - make_interval(days => $1)",
        days,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("DELETE FROM mod_ai_agent.usage WHERE day < current_date - 2")
        .execute(conn)
        .await?;
    Ok(messages)
}

pub async fn insert_proposal(
    conn: &mut PgConnection,
    user_id: Uuid,
    tool: &str,
    args: &str,
    facts: &str,
) -> Result<Proposal, sqlx::Error> {
    sqlx::query_as!(
        Proposal,
        "INSERT INTO mod_ai_agent.proposals (id, user_id, tool, args, facts)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, tool, args, facts, created_at",
        Uuid::now_v7(),
        user_id,
        tool,
        args,
        facts,
    )
    .fetch_one(conn)
    .await
}

/// The user's proposals that can still be confirmed: pending and less than a day old.
pub async fn pending_proposals(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<Proposal>, sqlx::Error> {
    sqlx::query_as!(
        Proposal,
        "SELECT id, tool, args, facts, created_at FROM mod_ai_agent.proposals
         WHERE user_id = $1 AND status = 'pending' AND created_at > now() - interval '1 day'
         ORDER BY id",
        user_id,
    )
    .fetch_all(conn)
    .await
}

/// Takes a pending proposal of this user for confirming: it becomes `done` and is returned, once.
/// `None` when it is not theirs, not pending, or older than a day. Run in the transaction that
/// runs the change, so a change that fails puts the proposal back.
pub async fn claim_proposal(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<Proposal>, sqlx::Error> {
    sqlx::query_as!(
        Proposal,
        "UPDATE mod_ai_agent.proposals SET status = 'done', decided_at = now()
         WHERE id = $1 AND user_id = $2 AND status = 'pending'
           AND created_at > now() - interval '1 day'
         RETURNING id, tool, args, facts, created_at",
        id,
        user_id,
    )
    .fetch_optional(conn)
    .await
}

/// The change of a proposal that can still be confirmed, without taking it: the confirm route
/// looks at it to know whether a temporary password must be made before its transaction opens.
pub async fn pending_args(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT args FROM mod_ai_agent.proposals
         WHERE id = $1 AND user_id = $2 AND status = 'pending'
           AND created_at > now() - interval '1 day'",
        id,
        user_id,
    )
    .fetch_optional(conn)
    .await
}

/// Marks a proposal whose change was refused as `failed`, so it is not offered again.
pub async fn fail_proposal(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE mod_ai_agent.proposals SET status = 'failed', decided_at = now()
         WHERE id = $1 AND user_id = $2 AND status = 'pending'",
        id,
        user_id,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The user turns a pending proposal down. `false` when there was nothing to turn down.
pub async fn reject_proposal(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let rejected = sqlx::query!(
        "UPDATE mod_ai_agent.proposals SET status = 'rejected', decided_at = now()
         WHERE id = $1 AND user_id = $2 AND status = 'pending'
         RETURNING id",
        id,
        user_id,
    )
    .fetch_optional(conn)
    .await?;
    Ok(rejected.is_some())
}
