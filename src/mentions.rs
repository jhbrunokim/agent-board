//! Project-scoped team directory and durable mention notifications.
use std::collections::{BTreeMap, BTreeSet};

use super::*;

const MAX_MENTIONS: usize = 32;
const MAX_MEMBERS: usize = 64;
type Tx<'a> = sqlx::Transaction<'a, sqlx::Any>;

pub(super) async fn migrate(tx: &mut Tx<'_>) -> std::result::Result<(), sqlx::Error> {
    for statement in [
        "CREATE TABLE mention_teams (project_id TEXT NOT NULL REFERENCES projects(id), team TEXT NOT NULL, leader TEXT NOT NULL, version BIGINT NOT NULL, PRIMARY KEY(project_id,team))",
        "CREATE TABLE mention_members (project_id TEXT NOT NULL, handle TEXT NOT NULL, team TEXT NOT NULL, principal TEXT NOT NULL, PRIMARY KEY(project_id,handle), FOREIGN KEY(project_id,team) REFERENCES mention_teams(project_id,team))",
        "CREATE INDEX mention_members_team ON mention_members(project_id,team)",
        "CREATE TABLE leader_inbox (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), recipient TEXT NOT NULL, seq BIGINT NOT NULL, message_id TEXT NOT NULL REFERENCES messages(id), handles TEXT NOT NULL, acknowledged BIGINT NOT NULL DEFAULT 0, UNIQUE(project_id,recipient,seq))",
    ] {
        sqlx::query(statement).execute(&mut **tx).await?;
    }
    Ok(())
}

pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/mention-teams/{team}", get(team).post(configure))
        .route("/api/v1/inbox", get(inbox))
        .route("/api/v1/inbox/{id}/ack", post(acknowledge))
}

fn handle_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

fn valid_handle(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|c| handle_byte(c) && !c.is_ascii_uppercase())
}

fn valid_principal(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

// Recognize plain mentions at word boundaries and skip backtick-delimited code.
// Structured mentions are available for callers that need exact routing.
fn scan(text: &str, handles: &mut BTreeSet<String>) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut code = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            while bytes.get(i) == Some(&b'`') {
                i += 1;
            }
            let ticks = i - start;
            if code == 0 {
                code = ticks;
            } else if code == ticks {
                code = 0;
            }
            continue;
        }
        let boundary =
            i == 0 || bytes[i - 1].is_ascii_whitespace() || b"([{".contains(&bytes[i - 1]);
        if code == 0 && bytes[i] == b'@' && boundary {
            let start = i + 1;
            i = start;
            while bytes.get(i).is_some_and(|c| handle_byte(*c)) {
                i += 1;
            }
            let handle = &text[start..i];
            if valid_handle(handle) && !handles.contains(handle) {
                if handles.len() == MAX_MENTIONS {
                    return true;
                }
                handles.insert(handle.to_owned());
            }
        } else {
            i += 1;
        }
    }
    false
}

fn candidates(input: &Input) -> Result<(BTreeSet<String>, bool)> {
    let mut handles = BTreeSet::new();
    if let Some(explicit) = &input.mentions {
        if explicit.len() > MAX_MENTIONS || explicit.iter().any(|h| !valid_handle(h)) {
            return Err(invalid());
        }
        handles.extend(explicit.iter().cloned());
        return Ok((handles, false));
    }
    let text_fields = ["text", "body", "question", "answer"]
        .into_iter()
        .filter_map(|field| input.payload[field].as_str());
    for text in input.title.as_deref().into_iter().chain(text_fields) {
        if scan(text, &mut handles) {
            return Ok((handles, true));
        }
    }
    Ok((handles, false))
}

pub(super) async fn enqueue(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    message_id: &str,
    seq: i64,
    input: &Input,
) -> Result<(usize, bool)> {
    let (handles, truncated) = candidates(input)?;
    let mut recipients = BTreeMap::<String, Vec<String>>::new();
    for handle in handles {
        let row = sqlx::query(&s.sql(
            "SELECT t.leader FROM mention_members m JOIN mention_teams t ON t.project_id=m.project_id AND t.team=m.team WHERE m.project_id=? AND m.handle=?",
        ))
        .bind(&who.project)
        .bind(&handle)
        .fetch_optional(&mut **tx)
        .await?;
        match row {
            Some(row) => recipients
                .entry(row.try_get("leader")?)
                .or_default()
                .push(handle),
            None if input.mentions.is_some() => {
                return Err(Error(StatusCode::UNPROCESSABLE_ENTITY, "unknown_mention"));
            }
            None => {}
        }
    }
    for (recipient, handles) in &recipients {
        sqlx::query(&s.sql(
            "INSERT INTO leader_inbox(id,project_id,recipient,seq,message_id,handles) VALUES (?,?,?,?,?,?)",
        ))
        .bind(Uuid::new_v4().to_string())
        .bind(&who.project)
        .bind(recipient)
        .bind(seq)
        .bind(message_id)
        .bind(serde_json::to_string(handles).map_err(|_| invalid())?)
        .execute(&mut **tx)
        .await?;
    }
    Ok((recipients.len(), truncated))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Member {
    handle: String,
    principal: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TeamConfig {
    expected_version: i64,
    leader: String,
    members: Vec<Member>,
}

async fn lock_project(s: &Store, tx: &mut Tx<'_>, who: &Identity) -> Result<()> {
    sqlx::query(&s.sql("UPDATE projects SET head=head WHERE id=?"))
        .bind(&who.project)
        .execute(&mut **tx)
        .await?;
    s.active(who, tx).await
}

async fn configure(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(team): Path<String>,
    headers: HeaderMap,
    Json(input): Json<TeamConfig>,
) -> Result<Json<Value>> {
    let key = key(&headers)?;
    let handles: BTreeSet<_> = input.members.iter().map(|m| &m.handle).collect();
    if !valid_handle(&team)
        || !valid_principal(&input.leader)
        || input.expected_version < 0
        || input.members.len() > MAX_MEMBERS
        || handles.len() != input.members.len()
        || input
            .members
            .iter()
            .any(|m| !valid_handle(&m.handle) || !valid_principal(&m.principal))
        || key.is_empty()
        || key.len() > 128
    {
        return Err(invalid());
    }
    let operation = format!("mention:team:{team}");
    let digest = hash(&serde_json::to_string(&input).map_err(|_| invalid())?);
    let mut tx = s.pool.begin().await?;
    lock_project(&s, &mut tx, &who).await?;
    if let Some(prior) = sqlx::query(&s.sql(
        "SELECT hash,response FROM dedup WHERE project_id=? AND principal=? AND operation=? AND key=?",
    ))
    .bind(&who.project)
    .bind(&who.principal)
    .bind(&operation)
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.rollback().await?;
        if prior.try_get::<String, _>("hash")? != digest {
            return Err(conflict());
        }
        return Ok(Json(decode(&prior.try_get::<String, _>("response")?)?));
    }

    let current =
        sqlx::query(&s.sql("SELECT version FROM mention_teams WHERE project_id=? AND team=?"))
            .bind(&who.project)
            .bind(&team)
            .fetch_optional(&mut *tx)
            .await?
            .map(|row| row.try_get::<i64, _>("version"))
            .transpose()?
            .unwrap_or(0);
    if current != input.expected_version {
        return Err(Error(StatusCode::CONFLICT, "version_conflict"));
    }
    let principals: BTreeSet<_> = std::iter::once(&input.leader)
        .chain(input.members.iter().map(|m| &m.principal))
        .collect();
    for principal in principals {
        if sqlx::query(&s.sql(
            "SELECT hash FROM tokens WHERE project_id=? AND principal=? AND revoked=0 LIMIT 1",
        ))
        .bind(&who.project)
        .bind(principal)
        .fetch_optional(&mut *tx)
        .await?
        .is_none()
        {
            return Err(Error(StatusCode::UNPROCESSABLE_ENTITY, "unknown_principal"));
        }
    }
    let version = current.checked_add(1).ok_or_else(invalid)?;
    sqlx::query(&s.sql(
        "INSERT INTO mention_teams(project_id,team,leader,version) VALUES (?,?,?,?) ON CONFLICT(project_id,team) DO UPDATE SET leader=excluded.leader,version=excluded.version",
    ))
    .bind(&who.project).bind(&team).bind(&input.leader).bind(version)
    .execute(&mut *tx).await?;
    sqlx::query(&s.sql("DELETE FROM mention_members WHERE project_id=? AND team=?"))
        .bind(&who.project)
        .bind(&team)
        .execute(&mut *tx)
        .await?;
    for member in &input.members {
        let inserted = sqlx::query(&s.sql(
            "INSERT INTO mention_members(project_id,handle,team,principal) VALUES (?,?,?,?) ON CONFLICT(project_id,handle) DO NOTHING",
        ))
        .bind(&who.project).bind(&member.handle).bind(&team).bind(&member.principal)
        .execute(&mut *tx).await?.rows_affected();
        if inserted == 0 {
            return Err(Error(StatusCode::CONFLICT, "handle_already_registered"));
        }
    }
    let response =
        json!({"team":team,"leader":input.leader,"version":version,"members":input.members});
    sqlx::query(&s.sql(
        "INSERT INTO dedup(project_id,principal,operation,key,hash,response) VALUES (?,?,?,?,?,?)",
    ))
    .bind(&who.project)
    .bind(&who.principal)
    .bind(operation)
    .bind(key)
    .bind(digest)
    .bind(response.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(response))
}

async fn team(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(team): Path<String>,
) -> Result<Json<Value>> {
    if !valid_handle(&team) {
        return Err(invalid());
    }
    let rows = sqlx::query(&s.sql(
        "SELECT t.leader,t.version,m.handle,m.principal FROM mention_teams t LEFT JOIN mention_members m ON m.project_id=t.project_id AND m.team=t.team WHERE t.project_id=? AND t.team=? ORDER BY m.handle",
    ))
    .bind(&who.project).bind(&team).fetch_all(&s.pool).await?;
    let row = rows.first().ok_or_else(missing)?;
    let mut members = Vec::new();
    for r in &rows {
        if let Some(handle) = r.try_get::<Option<String>, _>("handle")? {
            members.push(Member {
                handle,
                principal: r.try_get("principal")?,
            });
        }
    }
    Ok(Json(json!({
        "team":team,"leader":row.try_get::<String,_>("leader")?,
        "version":row.try_get::<i64,_>("version")?,"members":members
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxPage {
    after: Option<String>,
    limit: Option<i64>,
    unread: Option<bool>,
}

async fn inbox(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(page): Query<InboxPage>,
) -> Result<Json<Value>> {
    let limit = page.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(invalid());
    }
    let unread = page.unread.unwrap_or(true);
    let scope = format!(
        "inbox:{}",
        hash(&json!([who.project, who.principal, unread]).to_string())
    );
    let after = parse_cursor(page.after.as_deref(), &scope)?;
    let unread_clause = if unread { "AND i.acknowledged=0" } else { "" };
    let sql = format!(
        "SELECT i.id,i.seq,i.message_id,i.handles,i.acknowledged,m.thread_id,m.sender FROM leader_inbox i JOIN messages m ON m.project_id=i.project_id AND m.id=i.message_id WHERE i.project_id=? AND i.recipient=? AND i.seq>? {unread_clause} ORDER BY i.seq LIMIT ?"
    );
    let rows = sqlx::query(&s.sql(&sql))
        .bind(&who.project)
        .bind(&who.principal)
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&s.pool)
        .await?;
    let has_more = rows.len() > limit as usize;
    let mut cursor = after;
    let mut items = Vec::new();
    for row in rows.into_iter().take(limit as usize) {
        cursor = row.try_get("seq")?;
        let thread_id: String = row.try_get("thread_id")?;
        let lookup = format!(
            "/api/v1/threads/{thread_id}/messages?limit=1&after={}:thread:{thread_id}:{}",
            who.project,
            cursor - 1
        );
        items.push(json!({
            "id":row.try_get::<String,_>("id")?,"sequence":cursor,
            "message_id":row.try_get::<String,_>("message_id")?,"thread_id":thread_id,
            "sender":row.try_get::<String,_>("sender")?,
            "mentions":decode(&row.try_get::<String,_>("handles")?)?,
            "acknowledged":row.try_get::<i64,_>("acknowledged")? != 0,"lookup":lookup
        }));
    }
    Ok(Json(
        json!({"items":items,"next_cursor":format!("{scope}:{cursor}"),"has_more":has_more}),
    ))
}

async fn acknowledge(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let mut tx = s.pool.begin().await?;
    lock_project(&s, &mut tx, &who).await?;
    let updated =
        sqlx::query(&s.sql(
            "UPDATE leader_inbox SET acknowledged=1 WHERE project_id=? AND recipient=? AND id=?",
        ))
        .bind(&who.project)
        .bind(&who.principal)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(missing());
    }
    tx.commit().await?;
    Ok(Json(json!({"id":id,"acknowledged":true})))
}

fn decode(s: &str) -> Result<Value> {
    serde_json::from_str(s)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"))
}
