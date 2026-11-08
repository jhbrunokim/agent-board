mod access;
mod task_context;
pub use access::Role;
mod projections;
mod sources;
pub mod vault;
mod workflows;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, Row, any::AnyPoolOptions};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pool: AnyPool,
    postgres: bool,
}
#[derive(Debug)]
pub struct Error(StatusCode, &'static str);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"code":self.1}))).into_response()
    }
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        eprintln!(
            "database operation failed: {}",
            match e {
                sqlx::Error::PoolTimedOut => "pool timeout",
                _ => "query failure",
            }
        );
        Self(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
    }
}
type Result<T> = std::result::Result<T, Error>;
fn invalid() -> Error {
    Error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input")
}
fn missing() -> Error {
    Error(StatusCode::NOT_FOUND, "not_found")
}
fn conflict() -> Error {
    Error(StatusCode::CONFLICT, "idempotency_conflict")
}

impl Store {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        sqlx::any::install_default_drivers();
        anyhow::ensure!(
            url.starts_with("sqlite:")
                || url.starts_with("postgres://")
                || url.starts_with("postgresql://"),
            "unsupported database scheme"
        );
        let postgres = !url.starts_with("sqlite:");
        let pool = AnyPoolOptions::new()
            .max_connections(if postgres { 8 } else { 1 })
            .after_connect(move |c, _| {
                Box::pin(async move {
                    if !postgres {
                        for s in [
                            "PRAGMA foreign_keys=ON",
                            "PRAGMA journal_mode=WAL",
                            "PRAGMA synchronous=FULL",
                            "PRAGMA busy_timeout=5000",
                        ] {
                            sqlx::query(s).execute(&mut *c).await?;
                        }
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await?;
        Ok(Self { pool, postgres })
    }
    fn sql(&self, s: &str) -> String {
        if !self.postgres {
            return s.into();
        }
        let mut n = 0;
        s.chars()
            .map(|c| {
                if c == '?' {
                    n += 1;
                    format!("${n}")
                } else {
                    c.to_string()
                }
            })
            .collect()
    }
    pub async fn migrate(&self) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version BIGINT PRIMARY KEY)")
            .execute(&mut *tx)
            .await?;
        let version: i64 =
            sqlx::query("SELECT COALESCE(MAX(version),0) AS version FROM schema_version")
                .fetch_one(&mut *tx)
                .await?
                .try_get("version")?;
        anyhow::ensure!(version <= 5, "database schema is newer than this server");
        for s in [
            "CREATE TABLE IF NOT EXISTS schema_version (version BIGINT PRIMARY KEY)",
            "CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, head BIGINT NOT NULL DEFAULT 0)",
            "CREATE TABLE IF NOT EXISTS tokens (hash TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), principal TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), title TEXT NOT NULL, UNIQUE(project_id,id))",
            "CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, thread_id TEXT NOT NULL, sender TEXT NOT NULL, seq BIGINT NOT NULL, payload TEXT NOT NULL, FOREIGN KEY(project_id,thread_id) REFERENCES threads(project_id,id))",
            "CREATE TABLE IF NOT EXISTS events (project_id TEXT NOT NULL REFERENCES projects(id), seq BIGINT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(project_id,seq))",
            "CREATE TABLE IF NOT EXISTS dedup (project_id TEXT NOT NULL, principal TEXT NOT NULL, operation TEXT NOT NULL, key TEXT NOT NULL, hash TEXT NOT NULL, response TEXT NOT NULL, PRIMARY KEY(project_id,principal,operation,key))",
            "CREATE INDEX IF NOT EXISTS messages_thread ON messages(project_id,thread_id)",
            "INSERT INTO schema_version(version) VALUES (1) ON CONFLICT DO NOTHING",
        ] {
            sqlx::query(s).execute(&mut *tx).await?;
        }
        if version < 2 {
            workflows::migrate(&mut tx).await?;
            sqlx::query("INSERT INTO schema_version(version) VALUES (2)")
                .execute(&mut *tx)
                .await?;
        }
        if version < 3 {
            sqlx::query("ALTER TABLE tokens ADD COLUMN role TEXT NOT NULL DEFAULT 'reviewer'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("ALTER TABLE tokens ADD COLUMN revoked BIGINT NOT NULL DEFAULT 0")
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO schema_version(version) VALUES (3)")
                .execute(&mut *tx)
                .await?;
        }
        if version < 4 {
            sources::migrate(&mut tx).await?;
            sqlx::query("INSERT INTO schema_version(version) VALUES (4)")
                .execute(&mut *tx)
                .await?;
        }
        if version < 5 {
            task_context::migrate(&mut tx).await?;
            sqlx::query("INSERT INTO schema_version(version) VALUES (5)")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn check_schema(&self) -> anyhow::Result<()> {
        let version: i64 =
            sqlx::query("SELECT COALESCE(MAX(version),0) AS version FROM schema_version")
                .fetch_one(&self.pool)
                .await?
                .try_get("version")?;
        anyhow::ensure!(
            version == 5,
            "schema version mismatch; run migrate before starting this server"
        );
        Ok(())
    }
    pub async fn provision(&self, name: &str, principal: &str) -> anyhow::Result<(String, String)> {
        anyhow::ensure!(
            !name.trim().is_empty() && !principal.trim().is_empty(),
            "name and principal required"
        );
        let id = Uuid::new_v4().to_string();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let mut tx = self.pool.begin().await?;
        sqlx::query(&self.sql("INSERT INTO projects(id,name) VALUES (?,?)"))
            .bind(&id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            &self.sql("INSERT INTO tokens(hash,project_id,principal,role) VALUES (?,?,?,'admin')"),
        )
        .bind(hash(&token))
        .bind(&id)
        .bind(principal)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((id, token))
    }
    pub async fn grant(&self, project: &str, principal: &str) -> anyhow::Result<String> {
        self.grant_role(project, principal, Role::Contributor).await
    }
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Identity> {
        let token = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(Error(StatusCode::UNAUTHORIZED, "unauthorized"))?;
        let row =
            sqlx::query(&self.sql(
                "SELECT project_id,principal,role,hash FROM tokens WHERE hash=? AND revoked=0",
            ))
            .bind(hash(token))
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error(StatusCode::UNAUTHORIZED, "unauthorized"))?;
        Ok(Identity {
            project: row.try_get("project_id")?,
            principal: row.try_get("principal")?,
            role: access::parse_role(&row.try_get::<String, _>("role")?)?,
            token_id: row.try_get("hash")?,
        })
    }
    async fn write(
        &self,
        who: &Identity,
        key: &str,
        thread: Option<&str>,
        input: &Input,
    ) -> Result<Value> {
        if key.is_empty() || key.len() > 128 || !input.payload.is_object() {
            return Err(invalid());
        }
        if thread.is_none()
            && input
                .title
                .as_ref()
                .is_none_or(|t| t.trim().is_empty() || t.len() > 500)
        {
            return Err(invalid());
        }
        let operation = thread
            .map(|id| format!("reply:{id}"))
            .unwrap_or("create".into());
        let digest = hash(&serde_json::to_string(input).map_err(|_| invalid())?);
        let mut tx = self.pool.begin().await?;
        // A project row lock serializes committed event order, including dedup retries.
        let seq: i64 =
            sqlx::query(&self.sql("UPDATE projects SET head=head+1 WHERE id=? RETURNING head"))
                .bind(&who.project)
                .fetch_one(&mut *tx)
                .await?
                .try_get("head")?;
        self.active(who, &mut tx).await?;
        let prior=sqlx::query(&self.sql("SELECT hash,response FROM dedup WHERE project_id=? AND principal=? AND operation=? AND key=?")).bind(&who.project).bind(&who.principal).bind(&operation).bind(key).fetch_optional(&mut *tx).await?;
        if let Some(row) = prior {
            tx.rollback().await?;
            if row.try_get::<String, _>("hash")? != digest {
                return Err(conflict());
            }
            return serde_json::from_str(&row.try_get::<String, _>("response")?)
                .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"));
        }
        let thread_id = thread
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if thread.is_some() {
            if sqlx::query(&self.sql("SELECT id FROM threads WHERE id=? AND project_id=?"))
                .bind(&thread_id)
                .bind(&who.project)
                .fetch_optional(&mut *tx)
                .await?
                .is_none()
            {
                return Err(missing());
            }
        } else {
            sqlx::query(&self.sql("INSERT INTO threads(id,project_id,title) VALUES (?,?,?)"))
                .bind(&thread_id)
                .bind(&who.project)
                .bind(input.title.as_deref().unwrap())
                .execute(&mut *tx)
                .await?;
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query(&self.sql(
            "INSERT INTO messages(id,project_id,thread_id,sender,seq,payload) VALUES (?,?,?,?,?,?)",
        ))
        .bind(&id)
        .bind(&who.project)
        .bind(&thread_id)
        .bind(&who.principal)
        .bind(seq)
        .bind(input.payload.to_string())
        .execute(&mut *tx)
        .await?;
        let response = json!({"message_id":id,"thread_id":thread_id,"sequence":seq});
        sqlx::query(&self.sql("INSERT INTO events(project_id,seq,body) VALUES (?,?,?)"))
            .bind(&who.project)
            .bind(seq)
            .bind(response.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(&self.sql("INSERT INTO dedup(project_id,principal,operation,key,hash,response) VALUES (?,?,?,?,?,?)")).bind(&who.project).bind(&who.principal).bind(operation).bind(key).bind(digest).bind(response.to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(response)
    }
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
#[derive(Clone)]
struct Identity {
    project: String,
    principal: String,
    role: Role,
    token_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub title: Option<String>,
    pub payload: Value,
}
async fn auth(State(s): State<Store>, mut request: Request, next: Next) -> Result<Response> {
    let who = s.authenticate(request.headers()).await?;
    if !access::allowed(&who, request.method().as_str(), request.uri().path()) {
        return Err(Error(StatusCode::FORBIDDEN, "permission_denied"));
    }
    request.extensions_mut().insert(who);
    Ok(next.run(request).await)
}
pub fn router(s: Store) -> Router {
    Router::new()
        .route("/api/v1/threads", post(create))
        .route("/api/v1/threads/{id}/messages", post(reply).get(read))
        .route("/api/v1/events", get(events))
        .route("/api/v1/capabilities", get(capabilities))
        .merge(workflows::routes())
        .merge(sources::routes())
        .merge(task_context::routes())
        .merge(projections::routes())
        .merge(access::routes())
        .route_layer(middleware::from_fn_with_state(s.clone(), auth))
        .route("/ready", get(ready))
        .route("/health", get(|| async { Json(json!({"status":"up"})) }))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(s)
}
fn key(h: &HeaderMap) -> Result<&str> {
    h.get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or(invalid())
}
async fn create(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    h: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    Ok(Json(s.write(&who, key(&h)?, None, &input).await?))
}
async fn reply(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    Ok(Json(s.write(&who, key(&h)?, Some(&id), &input).await?))
}
#[derive(Deserialize)]
struct Page {
    after: Option<String>,
    limit: Option<i64>,
}
async fn events(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    let scope = format!("{}:events", who.project);
    let after = parse_cursor(page.after.as_deref(), &scope)?;
    let limit = page.limit.unwrap_or(50);
    if after < 0 || !(1..=200).contains(&limit) {
        return Err(invalid());
    }
    let rows = sqlx::query(
        &s.sql("SELECT seq,body FROM events WHERE project_id=? AND seq>? ORDER BY seq LIMIT ?"),
    )
    .bind(&who.project)
    .bind(after)
    .bind(limit)
    .fetch_all(&s.pool)
    .await?;
    let mut items = Vec::new();
    let mut cursor = after;
    for r in rows {
        cursor = r.try_get("seq")?;
        items.push(
            serde_json::from_str::<Value>(&r.try_get::<String, _>("body")?)
                .map_err(|_| invalid())?,
        );
    }
    Ok(Json(
        json!({"items":items,"next_cursor":format!("{scope}:{cursor}"),"project_id":who.project}),
    ))
}
async fn read(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    let limit = page.limit.unwrap_or(50);
    let scope = format!("{}:thread:{}", who.project, id);
    let offset = parse_cursor(page.after.as_deref(), &scope)?;
    if !(1..=200).contains(&limit) || offset < 0 {
        return Err(invalid());
    }
    if sqlx::query(&s.sql("SELECT id FROM threads WHERE id=? AND project_id=?"))
        .bind(&id)
        .bind(&who.project)
        .fetch_optional(&s.pool)
        .await?
        .is_none()
    {
        return Err(missing());
    }
    let rows=sqlx::query(&s.sql("SELECT id,sender,payload,seq FROM messages WHERE thread_id=? AND project_id=? AND seq>? ORDER BY seq LIMIT ?")).bind(&id).bind(&who.project).bind(offset).bind(limit).fetch_all(&s.pool).await?;
    let mut items = Vec::new();
    let mut cursor = offset;
    for r in rows {
        cursor = r.try_get("seq")?;
        items.push(json!({"id":r.try_get::<String,_>("id")?,"sender":r.try_get::<String,_>("sender")?,"payload":serde_json::from_str::<Value>(&r.try_get::<String,_>("payload")?).map_err(|_|invalid())?}));
    }
    Ok(Json(
        json!({"items":items,"next_cursor":format!("{scope}:{cursor}")}),
    ))
}

fn parse_cursor(value: Option<&str>, scope: &str) -> Result<i64> {
    let Some(value) = value else {
        return Ok(0);
    };
    let (actual, number) = value.rsplit_once(':').ok_or_else(invalid)?;
    if actual != scope {
        return Err(Error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "cursor_scope_mismatch",
        ));
    }
    let number = number.parse::<i64>().map_err(|_| invalid())?;
    if number < 0 {
        return Err(invalid());
    }
    Ok(number)
}

async fn ready(State(s): State<Store>) -> Result<Json<Value>> {
    s.check_schema()
        .await
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "schema_not_ready"))?;
    Ok(Json(json!({"status":"ready","schema_version":5})))
}

async fn capabilities(axum::Extension(who): axum::Extension<Identity>) -> Json<Value> {
    Json(
        json!({"api_version":1,"schema_version":5,"project_id":who.project,"principal":who.principal,"features":["threads","events","tasks","claims","reviews","knowledge","sources","task_context","search","context","vault_export"],"instructions":"agent-board instructions","role":who.role,"scope":"project"}),
    )
}
