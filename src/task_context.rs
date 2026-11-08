//! Explicit, revision-pinned task context. Reads share one database snapshot.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
type Tx<'a> = sqlx::Transaction<'a, sqlx::Any>;

pub(super) async fn migrate(tx: &mut Tx<'_>) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("CREATE TABLE IF NOT EXISTS task_knowledge (project_id TEXT NOT NULL, task_id TEXT NOT NULL, knowledge_id TEXT NOT NULL, knowledge_content_revision BIGINT NOT NULL CHECK(knowledge_content_revision>0), knowledge_version BIGINT NOT NULL CHECK(knowledge_version>0), linked_by TEXT NOT NULL, PRIMARY KEY(project_id,task_id,knowledge_id), FOREIGN KEY(project_id,task_id) REFERENCES wf_objects(project_id,id), FOREIGN KEY(project_id,knowledge_id,knowledge_version) REFERENCES wf_object_history(project_id,id,version))")
        .execute(&mut **tx).await?;
    Ok(())
}
pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/tasks/{id}/knowledge", post(link))
        .route("/api/v1/tasks/{id}/context", get(read))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    expected_task_version: i64,
    run_id: String,
    claim_epoch: i64,
    knowledge_id: String,
    knowledge_content_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    max_chars: Option<usize>,
}
fn decode(s: &str) -> Result<Value> {
    serde_json::from_str(s)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"))
}
fn state_conflict(code: &'static str) -> Error {
    Error(StatusCode::CONFLICT, code)
}
fn bounded(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max
}
async fn object(s: &Store, tx: &mut Tx<'_>, who: &Identity, id: &str, kind: &str) -> Result<Value> {
    let row =
        sqlx::query(&s.sql("SELECT state FROM wf_objects WHERE project_id=? AND id=? AND kind=?"))
            .bind(&who.project)
            .bind(id)
            .bind(kind)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(missing)?;
    decode(&row.try_get::<String, _>("state")?)
}
async fn mutate(s: &Store, who: &Identity, id: &str, key: &str, input: Link) -> Result<Value> {
    if !bounded(key, 128)
        || !bounded(id, 128)
        || !bounded(&input.knowledge_id, 128)
        || !bounded(&input.run_id, 128)
        || input.expected_task_version < 1
        || input.knowledge_content_revision < 1
        || input.claim_epoch < 1
    {
        return Err(invalid());
    }
    let operation = format!("task:knowledge:link:{id}");
    let digest = hash(
        &serde_json::to_value(&input)
            .map_err(|_| invalid())?
            .to_string(),
    );
    let mut tx = s.pool.begin().await?;
    let sequence: i64 =
        sqlx::query(&s.sql("UPDATE projects SET head=head+1 WHERE id=? RETURNING head"))
            .bind(&who.project)
            .fetch_one(&mut *tx)
            .await?
            .try_get("head")?;
    s.active(who, &mut tx).await?;
    if let Some(prior) = sqlx::query(&s.sql("SELECT hash,response FROM dedup WHERE project_id=? AND principal=? AND operation=? AND key=?"))
        .bind(&who.project).bind(&who.principal).bind(&operation).bind(key).fetch_optional(&mut *tx).await? {
        tx.rollback().await?;
        if prior.try_get::<String,_>("hash")? != digest { return Err(conflict()); }
        return decode(&prior.try_get::<String,_>("response")?);
    }
    let mut task = object(s, &mut tx, who, id, "task").await?;
    if task["version"] != input.expected_task_version {
        return Err(state_conflict("version_conflict"));
    }
    let lease = &task["lease"];
    if lease.is_null() {
        return Err(state_conflict("claim_required"));
    }
    if lease["epoch"] != input.claim_epoch || task["lease_epoch"] != input.claim_epoch {
        return Err(state_conflict("stale_claim"));
    }
    if lease["principal"] != who.principal || lease["run_id"] != input.run_id {
        return Err(Error(StatusCode::FORBIDDEN, "forbidden"));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .ok_or(Error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clock_unavailable",
        ))?;
    if lease["expires_at_ms"].as_i64().ok_or_else(invalid)? <= now {
        return Err(state_conflict("claim_expired"));
    }
    if task["status"] == "done" {
        return Err(state_conflict("task_done"));
    }
    let knowledge = object(s, &mut tx, who, &input.knowledge_id, "knowledge").await?;
    if sqlx::query(&s.sql("SELECT knowledge_id FROM task_knowledge WHERE project_id=? AND task_id=? AND knowledge_id=?"))
        .bind(&who.project).bind(id).bind(&input.knowledge_id).fetch_optional(&mut *tx).await?.is_some() { return Err(state_conflict("knowledge_already_linked")); }
    let count: i64 = sqlx::query(
        &s.sql("SELECT COUNT(*) AS n FROM task_knowledge WHERE project_id=? AND task_id=?"),
    )
    .bind(&who.project)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?
    .try_get("n")?;
    if count >= 16 {
        return Err(state_conflict("knowledge_limit"));
    }
    if knowledge["content_revision"] != input.knowledge_content_revision {
        return Err(state_conflict("content_revision_conflict"));
    }
    let pinned_version = knowledge["version"].as_i64().ok_or_else(invalid)?;
    sqlx::query(&s.sql("INSERT INTO task_knowledge(project_id,task_id,knowledge_id,knowledge_content_revision,knowledge_version,linked_by) VALUES (?,?,?,?,?,?)"))
        .bind(&who.project).bind(id).bind(&input.knowledge_id).bind(input.knowledge_content_revision).bind(pinned_version).bind(&who.principal).execute(&mut *tx).await?;
    let next = input
        .expected_task_version
        .checked_add(1)
        .ok_or_else(invalid)?;
    task["version"] = json!(next);
    let state = task.to_string();
    sqlx::query(&s.sql(
        "UPDATE wf_objects SET version=?,state=? WHERE project_id=? AND id=? AND kind='task'",
    ))
    .bind(next)
    .bind(&state)
    .bind(&who.project)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        &s.sql("INSERT INTO wf_object_history(project_id,id,version,state) VALUES (?,?,?,?)"),
    )
    .bind(&who.project)
    .bind(id)
    .bind(next)
    .bind(state)
    .execute(&mut *tx)
    .await?;
    let response = json!({"data":{"task":task,"link":{"task_id":id,"knowledge_id":input.knowledge_id,"knowledge_content_revision":input.knowledge_content_revision,"knowledge_version":pinned_version,"linked_by":who.principal}},"sequence":sequence});
    let event = json!({"type":operation,"actor":who.principal,"entity_id":id,"knowledge_id":input.knowledge_id,"knowledge_content_revision":input.knowledge_content_revision,"sequence":sequence});
    sqlx::query(&s.sql("INSERT INTO events(project_id,seq,body) VALUES (?,?,?)"))
        .bind(&who.project)
        .bind(sequence)
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
async fn link(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Link>,
) -> Result<Json<Value>> {
    Ok(Json(mutate(&s, &who, &id, key(&h)?, input).await?))
}
async fn read(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    Query(o): Query<Options>,
) -> Result<Json<Value>> {
    build(s, who, id, o.max_chars.unwrap_or(8000)).await
}
pub(super) async fn build(
    s: Store,
    who: Identity,
    id: String,
    max_chars: usize,
) -> Result<Json<Value>> {
    if !(512..=32000).contains(&max_chars) || !bounded(&id, 128) {
        return Err(invalid());
    }
    let mut tx = s.pool.begin().await?;
    if s.postgres {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
    }
    s.active(&who, &mut tx).await?;
    let task = object(&s, &mut tx, &who, &id, "task").await?;
    let lookup = format!("/api/v1/tasks/{id}");
    let mut items = vec![json!({"kind":"task","id":id,"lookup":lookup,"data":task})];
    let revision = task["content_revision"].as_i64().ok_or_else(invalid)?;
    let reviews = sqlx::query(&s.sql("SELECT id,state FROM wf_reviews WHERE project_id=? AND subject_id=? AND subject_kind='task' AND subject_revision=? ORDER BY id LIMIT 17"))
        .bind(&who.project).bind(&id).bind(revision).fetch_all(&mut *tx).await?;
    let review_has_more = reviews.len() > 16;
    let mut overflow_reviews = Vec::new();
    for (index, row) in reviews.into_iter().enumerate() {
        let review = decode(&row.try_get::<String, _>("state")?)?;
        let rid = review["id"].as_str().ok_or_else(invalid)?;
        if index == 16 {
            overflow_reviews
                .push(json!({"kind":"review","id":rid,"lookup":format!("/api/v1/reviews/{rid}")}));
            break;
        }
        items.push(json!({"kind":"review","id":rid,"lookup":format!("/api/v1/reviews/{rid}"),"data":{"version":review["version"],"subject_revision":review["subject_revision"],"reviewer":review["reviewer"],"requester":review["requester"],"status":review["status"]}}));
    }
    let links = sqlx::query(&s.sql("SELECT l.knowledge_id,l.knowledge_content_revision,l.knowledge_version,l.linked_by,h.state AS pinned_state,o.state AS current_state FROM task_knowledge l JOIN wf_objects o ON o.project_id=l.project_id AND o.id=l.knowledge_id AND o.kind='knowledge' JOIN wf_object_history h ON h.project_id=l.project_id AND h.id=l.knowledge_id AND h.version=l.knowledge_version WHERE l.project_id=? AND l.task_id=? ORDER BY l.knowledge_id LIMIT 17"))
        .bind(&who.project).bind(&id).fetch_all(&mut *tx).await?;
    let knowledge_has_more = links.len() > 16;
    for row in links.into_iter().take(16) {
        let kid: String = row.try_get("knowledge_id")?;
        let pinned_revision: i64 = row.try_get("knowledge_content_revision")?;
        let pinned_version: i64 = row.try_get("knowledge_version")?;
        let pinned = decode(&row.try_get::<String, _>("pinned_state")?)?;
        let current = decode(&row.try_get::<String, _>("current_state")?)?;
        let mut data = pinned;
        data["sources"] =
            json!(sources::list_in_tx(&s, &mut tx, &who, &kid, pinned_revision).await?);
        items.push(json!({"kind":"knowledge","id":kid,"lookup":format!("/api/v1/knowledge/{kid}?version={pinned_version}"),"pinned_content_revision":pinned_revision,"current_content_revision":current["content_revision"],"stale":current["content_revision"] != pinned_revision,"pinned_version":pinned_version,"current_version":current["version"],"content_changed":current["content_revision"] != pinned_revision,"status_at_link":data["status"],"current_status":current["status"],"status_changed":data["status"] != current["status"],"linked_by":row.try_get::<String,_>("linked_by")?,"data":data}));
    }
    tx.commit().await?;
    // Charge the serialized array itself, including brackets, separators and JSON escaping.
    let mut selected = Vec::new();
    let mut used = 2;
    let mut omitted = Vec::new();
    let mut omitted_count = 0;
    let mut blocked = false;
    for item in items {
        let cost = item.to_string().chars().count() + usize::from(!selected.is_empty());
        if !blocked && used + cost <= max_chars {
            used += cost;
            selected.push(item);
        } else {
            blocked = true;
            omitted_count += 1;
            if omitted.len() < 8 {
                omitted.push(json!({"kind":item["kind"],"id":item["id"],"lookup":item["lookup"]}));
            }
        }
    }
    omitted_count += overflow_reviews.len() + usize::from(knowledge_has_more);
    for review in overflow_reviews {
        if omitted.len() < 8 {
            omitted.push(review);
        }
    }
    Ok(Json(
        json!({"project_id":who.project,"task_id":id,"items":selected,"content_chars":used,"max_chars":max_chars,"incomplete":omitted_count>0,"omitted_at_least":omitted_count,"omitted":omitted,"reviews_have_more":review_has_more,"next_cursor":null,"lookup":lookup}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn revoked_identity_cannot_replay_or_create_links() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::connect(&format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("revoked-context.db").display()
        ))
        .await
        .unwrap();
        s.migrate().await.unwrap();
        let (project, token) = s.provision("revoked context", "owner").await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let who = s.authenticate(&headers).await.unwrap();
        let task = json!({"id":"task","project_id":project,"version":2,"content_revision":1,"title":"Task","description":"Body","status":"in_progress","created_by":"owner","content_author":"owner","lease_epoch":1,"lease":{"principal":"owner","run_id":"run","epoch":1,"expires_at_ms":i64::MAX}});
        let knowledge = json!({"id":"knowledge","project_id":project,"version":1,"content_revision":1,"title":"Knowledge","body":"Body","status":"draft","created_by":"owner","content_author":"owner"});
        for (id, kind, version, state) in [
            ("task", "task", 2, task),
            ("knowledge", "knowledge", 1, knowledge),
        ] {
            sqlx::query(
                "INSERT INTO wf_objects(project_id,id,kind,version,state) VALUES (?,?,?,?,?)",
            )
            .bind(&project)
            .bind(id)
            .bind(kind)
            .bind(version as i64)
            .bind(state.to_string())
            .execute(&s.pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO wf_object_history(project_id,id,version,state) VALUES (?,?,?,?)",
            )
            .bind(&project)
            .bind(id)
            .bind(version as i64)
            .bind(state.to_string())
            .execute(&s.pool)
            .await
            .unwrap();
        }
        let input = || Link {
            expected_task_version: 2,
            run_id: "run".into(),
            claim_epoch: 1,
            knowledge_id: "knowledge".into(),
            knowledge_content_revision: 1,
        };
        let task_state: String =
            sqlx::query("SELECT state FROM wf_objects WHERE project_id=? AND id='task'")
                .bind(&project)
                .fetch_one(&s.pool)
                .await
                .unwrap()
                .try_get("state")
                .unwrap();
        let mut expired: Value = serde_json::from_str(&task_state).unwrap();
        expired["lease"]["expires_at_ms"] = json!(0);
        sqlx::query("UPDATE wf_objects SET state=? WHERE project_id=? AND id='task'")
            .bind(expired.to_string())
            .bind(&project)
            .execute(&s.pool)
            .await
            .unwrap();
        assert!(matches!(
            mutate(&s, &who, "task", "expired", input()).await,
            Err(Error(StatusCode::CONFLICT, "claim_expired"))
        ));
        sqlx::query("UPDATE wf_objects SET state=? WHERE project_id=? AND id='task'")
            .bind(task_state)
            .bind(&project)
            .execute(&s.pool)
            .await
            .unwrap();
        mutate(&s, &who, "task", "existing", input()).await.unwrap();
        s.revoke_token(&project, &who.token_id).await.unwrap();
        for key in ["existing", "new"] {
            assert!(matches!(
                mutate(&s, &who, "task", key, input()).await,
                Err(Error(StatusCode::UNAUTHORIZED, "unauthorized"))
            ));
        }
        let head: i64 = sqlx::query("SELECT head FROM projects WHERE id=?")
            .bind(&project)
            .fetch_one(&s.pool)
            .await
            .unwrap()
            .try_get("head")
            .unwrap();
        assert_eq!(head, 1);
    }
}
