//! Immutable source references and evidence attached to exact knowledge content revisions.
//! URIs are stored as references; the service never resolves or fetches them.
use super::*;
type Tx<'a> = sqlx::Transaction<'a, sqlx::Any>;

pub(super) async fn migrate(tx: &mut Tx<'_>) -> std::result::Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS sources (project_id TEXT NOT NULL REFERENCES projects(id), id TEXT NOT NULL, uri TEXT NOT NULL, revision TEXT NOT NULL, label TEXT NOT NULL, created_by TEXT NOT NULL, PRIMARY KEY(project_id,id))",
        "CREATE TABLE IF NOT EXISTS knowledge_sources (project_id TEXT NOT NULL, knowledge_id TEXT NOT NULL, content_revision BIGINT NOT NULL CHECK(content_revision>0), source_id TEXT NOT NULL, attached_by TEXT NOT NULL, PRIMARY KEY(project_id,knowledge_id,content_revision,source_id), FOREIGN KEY(project_id,knowledge_id) REFERENCES wf_objects(project_id,id), FOREIGN KEY(project_id,source_id) REFERENCES sources(project_id,id))",
    ] {
        sqlx::query(sql).execute(&mut **tx).await?;
    }
    Ok(())
}
pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/sources", post(create))
        .route("/api/v1/sources/{id}", get(read))
        .route("/api/v1/knowledge/{id}/sources", post(attach))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSource {
    uri: String,
    revision: String,
    label: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    expected_version: i64,
    source_id: String,
    expected_content_revision: i64,
}
enum Command {
    Create(NewSource),
    Attach(String, Attachment),
}
fn bounded(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn decode(s: &str) -> Result<Value> {
    serde_json::from_str(s)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"))
}
fn state_conflict(code: &'static str) -> Error {
    Error(StatusCode::CONFLICT, code)
}
// Source identifiers pin an immutable (uri, revision, label) record. Corrections require
// a new source, followed by an attachment that creates a new knowledge revision.
async fn mutate(s: &Store, who: &Identity, key: &str, command: Command) -> Result<Value> {
    if !bounded(key, 128) {
        return Err(invalid());
    }
    let (operation, input) = match &command {
        Command::Create(input) => ("source:create".into(), serde_json::to_value(input)),
        Command::Attach(id, input) => (format!("source:attach:{id}"), serde_json::to_value(input)),
    };
    let digest = hash(&input.map_err(|_| invalid())?.to_string());
    let mut tx = s.pool.begin().await?;
    let seq: i64 = sqlx::query(&s.sql("UPDATE projects SET head=head+1 WHERE id=? RETURNING head"))
        .bind(&who.project)
        .fetch_one(&mut *tx)
        .await?
        .try_get("head")?;
    s.active(who, &mut tx).await?;
    let prior = sqlx::query(&s.sql("SELECT hash,response FROM dedup WHERE project_id=? AND principal=? AND operation=? AND key=?"))
        .bind(&who.project).bind(&who.principal).bind(&operation).bind(key).fetch_optional(&mut *tx).await?;
    if let Some(row) = prior {
        tx.rollback().await?;
        if row.try_get::<String, _>("hash")? != digest {
            return Err(conflict());
        }
        return decode(&row.try_get::<String, _>("response")?);
    }
    let (data, entity_id, source_id) = match command {
        Command::Create(input) => {
            // Require an absolute URI scheme without restricting which reference systems
            // callers may use (https, file, git, urn, etc.). No network operation follows.
            let scheme = input.uri.split_once(':').map(|(scheme, _)| scheme);
            if !bounded(&input.uri, 4096)
                || !bounded(&input.revision, 256)
                || !bounded(&input.label, 500)
                || scheme.is_none_or(|scheme| {
                    scheme.is_empty()
                        || !scheme.as_bytes()[0].is_ascii_alphabetic()
                        || !scheme
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
                })
            {
                return Err(invalid());
            }
            let id = Uuid::new_v4().to_string();
            sqlx::query(&s.sql("INSERT INTO sources(project_id,id,uri,revision,label,created_by) VALUES (?,?,?,?,?,?)"))
                .bind(&who.project).bind(&id).bind(&input.uri).bind(&input.revision).bind(&input.label).bind(&who.principal).execute(&mut *tx).await?;
            (
                json!({"id":id,"project_id":who.project,"uri":input.uri,"revision":input.revision,"label":input.label,"created_by":who.principal}),
                id.clone(),
                id,
            )
        }
        Command::Attach(id, input) => {
            if input.expected_version < 1
                || input.expected_content_revision < 1
                || !bounded(&input.source_id, 128)
            {
                return Err(invalid());
            }
            let row = sqlx::query(&s.sql(
                "SELECT state FROM wf_objects WHERE project_id=? AND id=? AND kind='knowledge'",
            ))
            .bind(&who.project)
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(missing)?;
            let mut knowledge = decode(&row.try_get::<String, _>("state")?)?;
            let current_version = knowledge["version"].as_i64().ok_or_else(invalid)?;
            let current_revision = knowledge["content_revision"].as_i64().ok_or_else(invalid)?;
            if current_version != input.expected_version {
                return Err(state_conflict("version_conflict"));
            }
            if current_revision != input.expected_content_revision {
                return Err(state_conflict("content_revision_conflict"));
            }
            if sqlx::query(&s.sql("SELECT id FROM sources WHERE project_id=? AND id=?"))
                .bind(&who.project)
                .bind(&input.source_id)
                .fetch_optional(&mut *tx)
                .await?
                .is_none()
            {
                return Err(missing());
            }
            if sqlx::query(&s.sql("SELECT source_id FROM knowledge_sources WHERE project_id=? AND knowledge_id=? AND content_revision=? AND source_id=?"))
                .bind(&who.project).bind(&id).bind(current_revision).bind(&input.source_id).fetch_optional(&mut *tx).await?.is_some() {
                return Err(state_conflict("source_already_attached"));
            }
            let count: i64 = sqlx::query(&s.sql("SELECT COUNT(*) AS n FROM knowledge_sources WHERE project_id=? AND knowledge_id=? AND content_revision=?"))
                .bind(&who.project).bind(&id).bind(current_revision).fetch_one(&mut *tx).await?.try_get("n")?;
            if count >= 16 {
                return Err(state_conflict("source_limit"));
            }
            let next_version = current_version.checked_add(1).ok_or_else(invalid)?;
            let next_revision = current_revision.checked_add(1).ok_or_else(invalid)?;
            knowledge["version"] = json!(next_version);
            knowledge["content_revision"] = json!(next_revision);
            knowledge["status"] = json!("draft");
            copy_revision(s, &mut tx, who, &id, current_revision, next_revision).await?;
            sqlx::query(&s.sql("INSERT INTO knowledge_sources(project_id,knowledge_id,content_revision,source_id,attached_by) VALUES (?,?,?,?,?)"))
                .bind(&who.project).bind(&id).bind(next_revision).bind(&input.source_id).bind(&who.principal).execute(&mut *tx).await?;
            let state = knowledge.to_string();
            sqlx::query(&s.sql("UPDATE wf_objects SET version=?,state=? WHERE project_id=? AND id=? AND kind='knowledge'"))
                .bind(next_version).bind(&state).bind(&who.project).bind(&id).execute(&mut *tx).await?;
            sqlx::query(&s.sql(
                "INSERT INTO wf_object_history(project_id,id,version,state) VALUES (?,?,?,?)",
            ))
            .bind(&who.project)
            .bind(&id)
            .bind(next_version)
            .bind(state)
            .execute(&mut *tx)
            .await?;
            // The response presents the same evidence list as GET knowledge.
            let evidence = list_in_tx(s, &mut tx, who, &id, next_revision).await?;
            knowledge["sources"] = json!(evidence);
            (knowledge, id, input.source_id)
        }
    };
    let response = json!({"data":data,"sequence":seq});
    let event = json!({"type":operation,"actor":who.principal,"entity_id":entity_id,"source_id":source_id,"sequence":seq});
    sqlx::query(&s.sql("INSERT INTO events(project_id,seq,body) VALUES (?,?,?)"))
        .bind(&who.project)
        .bind(seq)
        .bind(event.to_string())
        .execute(&mut *tx)
        .await?;
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
    Ok(response)
}
// Call before any post-rollout approved knowledge transition. Existing stored verified
// versions are not rewritten or invalidated by migration.
pub(super) async fn require_evidence(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    revision: i64,
) -> Result<()> {
    let row = sqlx::query(&s.sql("SELECT source_id FROM knowledge_sources WHERE project_id=? AND knowledge_id=? AND content_revision=? LIMIT 1"))
        .bind(&who.project).bind(id).bind(revision).fetch_optional(&mut **tx).await?;
    if row.is_none() {
        return Err(state_conflict("sources_required"));
    }
    Ok(())
}
// Evidence editors remain contributors when their links are copied to a new revision.
pub(super) async fn require_independent_evidence(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    revision: i64,
    reviewer: &str,
) -> Result<()> {
    let row = sqlx::query(&s.sql("SELECT source_id FROM knowledge_sources WHERE project_id=? AND knowledge_id=? AND content_revision=? AND attached_by=? LIMIT 1"))
        .bind(&who.project).bind(id).bind(revision).bind(reviewer).fetch_optional(&mut **tx).await?;
    if row.is_some() {
        return Err(state_conflict("independent_reviewer_required"));
    }
    Ok(())
}
pub(super) async fn copy_revision(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    from: i64,
    to: i64,
) -> Result<()> {
    sqlx::query(&s.sql("INSERT INTO knowledge_sources(project_id,knowledge_id,content_revision,source_id,attached_by) SELECT project_id,knowledge_id,?,source_id,attached_by FROM knowledge_sources WHERE project_id=? AND knowledge_id=? AND content_revision=?"))
        .bind(to).bind(&who.project).bind(id).bind(from).execute(&mut **tx).await?;
    Ok(())
}
fn source(row: &sqlx::any::AnyRow) -> Result<Value> {
    Ok(
        json!({"id":row.try_get::<String,_>("id")?,"project_id":row.try_get::<String,_>("project_id")?,"uri":row.try_get::<String,_>("uri")?,"revision":row.try_get::<String,_>("revision")?,"label":row.try_get::<String,_>("label")?,"created_by":row.try_get::<String,_>("created_by")?}),
    )
}
const LIST: &str = "SELECT s.* FROM sources s JOIN knowledge_sources l ON s.project_id=l.project_id AND s.id=l.source_id WHERE l.project_id=? AND l.knowledge_id=? AND l.content_revision=? ORDER BY s.id";
pub(super) async fn list(s: &Store, who: &Identity, id: &str, revision: i64) -> Result<Vec<Value>> {
    sqlx::query(&s.sql(LIST))
        .bind(&who.project)
        .bind(id)
        .bind(revision)
        .fetch_all(&s.pool)
        .await?
        .iter()
        .map(source)
        .collect()
}
async fn list_in_tx(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    revision: i64,
) -> Result<Vec<Value>> {
    sqlx::query(&s.sql(LIST))
        .bind(&who.project)
        .bind(id)
        .bind(revision)
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(source)
        .collect()
}
async fn read(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let row = sqlx::query(&s.sql("SELECT * FROM sources WHERE project_id=? AND id=?"))
        .bind(&who.project)
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(missing)?;
    Ok(Json(json!({"data":source(&row)?})))
}
async fn create(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    h: HeaderMap,
    Json(input): Json<NewSource>,
) -> Result<Json<Value>> {
    Ok(Json(
        mutate(&s, &who, key(&h)?, Command::Create(input)).await?,
    ))
}
async fn attach(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Attachment>,
) -> Result<Json<Value>> {
    Ok(Json(
        mutate(&s, &who, key(&h)?, Command::Attach(id, input)).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn revoked_authenticated_writer_cannot_create_or_replay_source() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(&format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("revoke.db").display()
        ))
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let (project, token) = store.provision("revocation", "author").await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let stale = store.authenticate(&headers).await.unwrap();
        let command = || {
            Command::Create(NewSource {
                uri: "urn:evidence:revoked".into(),
                revision: "1".into(),
                label: "Evidence".into(),
            })
        };
        mutate(&store, &stale, "existing", command()).await.unwrap();
        store.revoke_token(&project, &stale.token_id).await.unwrap();
        for key in ["existing", "new"] {
            assert!(matches!(
                mutate(&store, &stale, key, command()).await,
                Err(Error(StatusCode::UNAUTHORIZED, "unauthorized"))
            ));
        }
        let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM sources")
            .fetch_one(&store.pool)
            .await
            .unwrap()
            .try_get("n")
            .unwrap();
        let head: i64 = sqlx::query(&store.sql("SELECT head FROM projects WHERE id=?"))
            .bind(project)
            .fetch_one(&store.pool)
            .await
            .unwrap()
            .try_get("head")
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(head, 1);
    }
    #[tokio::test]
    async fn schema_four_preserves_legacy_verified_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(&format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("legacy.db").display()
        ))
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let (project, _) = store.provision("legacy", "author").await.unwrap();
        let state = json!({"id":"legacy", "project_id":project,"version":1,"content_revision":1,"title":"Historical", "body":"Legacy evidence in prose", "status":"verified", "created_by":"author", "content_author":"author"}).to_string();
        sqlx::query(&store.sql("INSERT INTO wf_objects(project_id,id,kind,version,state) VALUES (?,'legacy','knowledge',1,?)")).bind(&project).bind(&state).execute(&store.pool).await.unwrap();
        sqlx::query(&store.sql(
            "INSERT INTO wf_object_history(project_id,id,version,state) VALUES (?,'legacy',1,?)",
        ))
        .bind(&project)
        .bind(&state)
        .execute(&store.pool)
        .await
        .unwrap();
        // Reconstruct the pre-rollout schema marker and tables, retaining v3 history.
        for sql in [
            "DROP TABLE knowledge_sources",
            "DROP TABLE sources",
            "DELETE FROM schema_version WHERE version=4",
        ] {
            sqlx::query(sql).execute(&store.pool).await.unwrap();
        }
        store.migrate().await.unwrap();
        let saved: String = sqlx::query(&store.sql(
            "SELECT state FROM wf_object_history WHERE project_id=? AND id='legacy' AND version=1",
        ))
        .bind(project)
        .fetch_one(&store.pool)
        .await
        .unwrap()
        .try_get("state")
        .unwrap();
        assert_eq!(saved, state);
    }
}
