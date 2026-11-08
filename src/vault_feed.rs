//! Incremental, read-only vault projection. Cursors bind project and renderer version.
use super::*;
use std::collections::BTreeSet;
type Tx<'a> = sqlx::Transaction<'a, sqlx::Any>;
const MAX_CHARS: usize = 128_000;
const CURSOR_VERSION: &str = "vault-v2";

pub(super) fn routes() -> Router<Store> {
    Router::new().route("/api/v1/vault", get(read))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    after: Option<String>,
    limit: Option<i64>,
}
enum Cursor {
    Bootstrap { head: i64, key: String },
    Delta { after: i64, target: Option<i64> },
}
fn parse(value: Option<&str>, project: &str) -> Result<Option<Cursor>> {
    let Some(value) = value else { return Ok(None) };
    if value.len() > 512 {
        return Err(invalid());
    }
    let parts: Vec<&str> = value.split(':').collect();
    if parts.first() != Some(&CURSOR_VERSION) {
        return Err(Error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "cursor_renderer_mismatch",
        ));
    }
    if parts.get(1) != Some(&project) {
        return Err(Error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "cursor_scope_mismatch",
        ));
    }
    let sequence = parts
        .get(3)
        .ok_or_else(invalid)?
        .parse::<i64>()
        .map_err(|_| invalid())?;
    if sequence < 0 {
        return Err(invalid());
    }
    match parts.get(2).copied() {
        Some("delta") if parts.len() == 4 || parts.len() == 5 => {
            let target = parts
                .get(4)
                .map(|v| v.parse::<i64>().map_err(|_| invalid()))
                .transpose()?;
            if target.is_some_and(|target| target < sequence) {
                return Err(invalid());
            }
            Ok(Some(Cursor::Delta {
                after: sequence,
                target,
            }))
        }
        Some("bootstrap") if parts.len() == 5 && valid_key(parts[4]) => {
            Ok(Some(Cursor::Bootstrap {
                head: sequence,
                key: parts[4].into(),
            }))
        }
        _ => Err(invalid()),
    }
}
fn valid_key(key: &str) -> bool {
    let Some((kind, id)) = key.split_once('-') else {
        return false;
    };
    matches!(kind, "object" | "thread" | "source" | "review") && Uuid::parse_str(id).is_ok()
}
fn decode(value: &str) -> Result<Value> {
    serde_json::from_str(value)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"))
}
fn reference(out: &mut BTreeSet<String>, event: &Value, field: &str, prefix: &str) {
    if let Some(id) = event[field].as_str() {
        out.insert(format!("{prefix}-{id}"));
    }
}
fn references(event: &Value, out: &mut BTreeSet<String>) {
    // Legacy create/reply events have no type. Always inspect thread_id so new
    // message event types cannot accidentally hide thread replies.
    reference(out, event, "thread_id", "thread");
    let operation = event["type"].as_str().unwrap_or("");
    if operation.starts_with("wf:review:") {
        reference(out, event, "entity_id", "review");
        reference(out, event, "review_id", "review");
        reference(out, event, "subject_id", "object");
    } else if operation.starts_with("wf:") || operation.starts_with("task:knowledge:link:") {
        reference(out, event, "entity_id", "object");
    } else if operation.starts_with("source:") {
        reference(out, event, "source_id", "source");
        // The operation includes the knowledge ID after source:attach:.
        if operation == "source:attach" || operation.starts_with("source:attach:") {
            reference(out, event, "entity_id", "object");
        }
    }
    if let Some(data) = event.get("data") {
        reference(out, data, "thread_id", "thread");
        if operation.starts_with("wf:review:") {
            reference(out, data, "id", "review");
            reference(out, &data["review"], "id", "review");
            reference(out, &data["subject"], "id", "object");
        } else if operation.starts_with("wf:") {
            reference(out, data, "id", "object");
        } else if operation.starts_with("source:") {
            reference(out, data, "source_id", "source");
            if operation.starts_with("source:attach") {
                reference(out, data, "id", "object");
            } else {
                reference(out, data, "id", "source");
            }
        } else if operation.starts_with("task:knowledge:link") {
            reference(out, &data["task"], "id", "object");
        }
    }
}
async fn document(s: &Store, tx: &mut Tx<'_>, who: &Identity, key: &str) -> Result<Option<Value>> {
    let (prefix, id) = key.split_once('-').ok_or_else(invalid)?;
    let (kind, mut data, lookup) = match prefix {
        "object" => {
            let Some(row) = sqlx::query(
                &s.sql("SELECT kind,state FROM wf_objects WHERE project_id=? AND id=?"),
            )
            .bind(&who.project)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?
            else {
                return Ok(None);
            };
            let kind: String = row.try_get("kind")?;
            let mut data = decode(&row.try_get::<String, _>("state")?)?;
            if kind == "knowledge" {
                let revision = data["content_revision"].as_i64().ok_or_else(invalid)?;
                data["sources"] = json!(sources::list_in_tx(s, tx, who, id, revision).await?);
            } else if kind == "task" {
                let rows = sqlx::query(&s.sql("SELECT knowledge_id,knowledge_content_revision,knowledge_version,linked_by FROM task_knowledge WHERE project_id=? AND task_id=? ORDER BY knowledge_id"))
                    .bind(&who.project).bind(id).fetch_all(&mut **tx).await?;
                let mut links = Vec::new();
                for row in rows {
                    let kid: String = row.try_get("knowledge_id")?;
                    let version: i64 = row.try_get("knowledge_version")?;
                    links.push(json!({"knowledge_id":kid,"knowledge_content_revision":row.try_get::<i64,_>("knowledge_content_revision")?,"knowledge_version":version,"linked_by":row.try_get::<String,_>("linked_by")?,"lookup":format!("/api/v1/knowledge/{kid}?version={version}")}));
                }
                data["knowledge_links"] = json!(links);
            }
            let collection = if kind == "task" { "tasks" } else { "knowledge" };
            (kind, data, format!("/api/v1/{collection}/{id}"))
        }
        "review" => {
            let Some(row) =
                sqlx::query(&s.sql("SELECT state FROM wf_reviews WHERE project_id=? AND id=?"))
                    .bind(&who.project)
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await?
            else {
                return Ok(None);
            };
            (
                "review".into(),
                decode(&row.try_get::<String, _>("state")?)?,
                format!("/api/v1/reviews/{id}"),
            )
        }
        "source" => {
            let Some(row) = sqlx::query(&s.sql(
                "SELECT uri,revision,label,created_by FROM sources WHERE project_id=? AND id=?",
            ))
            .bind(&who.project)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?
            else {
                return Ok(None);
            };
            (
                "source".into(),
                json!({"id":id,"project_id":who.project,"uri":row.try_get::<String,_>("uri")?,"revision":row.try_get::<String,_>("revision")?,"label":row.try_get::<String,_>("label")?,"created_by":row.try_get::<String,_>("created_by")?}),
                format!("/api/v1/sources/{id}"),
            )
        }
        "thread" => {
            let Some(row) =
                sqlx::query(&s.sql("SELECT title FROM threads WHERE project_id=? AND id=?"))
                    .bind(&who.project)
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await?
            else {
                return Ok(None);
            };
            let rows = sqlx::query(&s.sql("SELECT id,sender,seq,payload FROM messages WHERE project_id=? AND thread_id=? ORDER BY seq DESC LIMIT 51"))
                .bind(&who.project).bind(id).fetch_all(&mut **tx).await?;
            let earlier = rows.len() > 50;
            let mut messages = Vec::new();
            for row in rows.into_iter().take(50) {
                messages.push(json!({"id":row.try_get::<String,_>("id")?,"sender":row.try_get::<String,_>("sender")?,"seq":row.try_get::<i64,_>("seq")?,"payload":decode(&row.try_get::<String,_>("payload")?)?}));
            }
            messages.reverse();
            let sequence = messages.last().and_then(|m| m["seq"].as_i64()).unwrap_or(0);
            let lookup = format!("/api/v1/threads/{id}/messages");
            (
                "thread".into(),
                json!({"id":id,"project_id":who.project,"title":row.try_get::<String,_>("title")?,"messages":messages,"truncated":earlier,"incomplete":earlier,"earlier_messages_omitted":earlier,"lookup":lookup,"next_cursor":format!("{}:thread:{id}:{sequence}",who.project)}),
                lookup,
            )
        }
        _ => return Err(invalid()),
    };
    let incomplete = data["incomplete"] == true;
    data["lookup"] = json!(lookup);
    Ok(Some(
        json!({"id":key,"kind":kind,"data":data,"lookup":lookup,"incomplete":incomplete}),
    ))
}
// Every document is represented, even when its body does not fit. A metadata-only
// placeholder explicitly points to the full API record; advancing never hides an omission.
fn bounded_document(mut doc: Value, used: &mut usize, remaining: usize) -> Value {
    let cost = doc.to_string().chars().count() + 1;
    // Reserve room for every remaining metadata placeholder (at most 150 docs).
    if *used + cost + remaining * 768 > MAX_CHARS {
        let original = &doc["data"];
        let mut metadata = serde_json::Map::new();
        for field in ["id", "version", "content_revision"] {
            if let Some(value) = original.get(field) {
                metadata.insert(field.into(), value.clone());
            }
        }
        metadata.insert("lookup".into(), doc["lookup"].clone());
        metadata.insert("incomplete".into(), json!(true));
        metadata.insert("truncated".into(), json!(true));
        metadata.insert("omission_reason".into(), json!("page_content_budget"));
        doc["data"] = Value::Object(metadata);
        doc["incomplete"] = json!(true);
    }
    *used += doc.to_string().chars().count() + 1;
    doc
}
async fn read(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(o): Query<Options>,
) -> Result<Json<Value>> {
    let limit = o.limit.unwrap_or(50);
    if !(1..=50).contains(&limit) {
        return Err(invalid());
    }
    let cursor = parse(o.after.as_deref(), &who.project)?;
    let mut tx = s.pool.begin().await?;
    if s.postgres {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
    }
    s.active(&who, &mut tx).await?;
    let head: i64 = sqlx::query(&s.sql("SELECT head FROM projects WHERE id=?"))
        .bind(&who.project)
        .fetch_one(&mut *tx)
        .await?
        .try_get("head")?;
    let cursor = cursor.unwrap_or(Cursor::Bootstrap {
        head,
        key: String::new(),
    });
    let mut keys = BTreeSet::new();
    let (next_cursor, has_more) = match cursor {
        Cursor::Bootstrap {
            head: captured,
            key,
        } => {
            if captured > head {
                return Err(invalid());
            }
            let rows = sqlx::query(&s.sql("SELECT doc_key FROM (SELECT 'object-' || id AS doc_key FROM wf_objects WHERE project_id=? UNION ALL SELECT 'thread-' || id AS doc_key FROM threads WHERE project_id=? UNION ALL SELECT 'source-' || id AS doc_key FROM sources WHERE project_id=? UNION ALL SELECT 'review-' || id AS doc_key FROM wf_reviews WHERE project_id=?) d WHERE doc_key>? ORDER BY doc_key LIMIT ?"))
                .bind(&who.project).bind(&who.project).bind(&who.project).bind(&who.project).bind(key).bind(limit+1).fetch_all(&mut *tx).await?;
            let more = rows.len() > limit as usize;
            for row in rows.into_iter().take(limit as usize) {
                keys.insert(row.try_get::<String, _>("doc_key")?);
            }
            let next = if more {
                format!(
                    "{CURSOR_VERSION}:{}:bootstrap:{captured}:{}",
                    who.project,
                    keys.last().ok_or_else(invalid)?
                )
            } else {
                if head > captured {
                    format!("{CURSOR_VERSION}:{}:delta:{captured}:{head}", who.project)
                } else {
                    format!("{CURSOR_VERSION}:{}:delta:{captured}", who.project)
                }
            };
            (next, more || head > captured)
        }
        Cursor::Delta { after, target } => {
            let target = target.unwrap_or(head);
            if after > head || target > head {
                return Err(invalid());
            }
            let rows = sqlx::query(&s.sql("SELECT seq,body FROM events WHERE project_id=? AND seq>? AND seq<=? ORDER BY seq LIMIT ?"))
                .bind(&who.project).bind(after).bind(target).bind(limit+1).fetch_all(&mut *tx).await?;
            let more = rows.len() > limit as usize;
            let mut scanned = after;
            for row in rows.into_iter().take(limit as usize) {
                scanned = row.try_get("seq")?;
                references(&decode(&row.try_get::<String, _>("body")?)?, &mut keys);
            }
            (
                if more {
                    format!("{CURSOR_VERSION}:{}:delta:{scanned}:{target}", who.project)
                } else {
                    format!("{CURSOR_VERSION}:{}:delta:{target}", who.project)
                },
                more,
            )
        }
    };
    let mut items = Vec::new();
    let mut used = 2;
    let total = keys.len();
    for (index, key) in keys.into_iter().enumerate() {
        if let Some(doc) = document(&s, &mut tx, &who, &key).await? {
            items.push(bounded_document(doc, &mut used, total - index - 1));
        }
    }
    tx.commit().await?;
    let incomplete = items.iter().any(|item| item["incomplete"] == true);
    Ok(Json(
        json!({"project_id":who.project,"items":items,"next_cursor":next_cursor,"has_more":has_more,"renderer_version":2,"incomplete":incomplete,"content_chars":used,"max_chars":MAX_CHARS}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_response_events_keep_document_references() {
        let mut keys = BTreeSet::new();
        references(
            &json!({"type":"wf:review:verdict:r", "data":{"review":{"id":"r"},"subject":{"id":"k"}}}),
            &mut keys,
        );
        assert_eq!(keys, BTreeSet::from(["review-r".into(), "object-k".into()]));
        keys.clear();
        references(
            &json!({"type":"future:message", "thread_id":"t"}),
            &mut keys,
        );
        assert_eq!(keys, BTreeSet::from(["thread-t".into()]));
    }
    #[tokio::test]
    async fn unrelated_events_advance_without_creating_documents_or_writes() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::connect(&format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("unrelated.db").display()
        ))
        .await
        .unwrap();
        s.migrate().await.unwrap();
        let (project, token) = s.provision("feed", "reader").await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let who = s.authenticate(&headers).await.unwrap();
        sqlx::query(
            "INSERT INTO events(project_id,seq,body) VALUES (?,1,'{\"type\":\"unrelated\"}')",
        )
        .bind(&project)
        .execute(&s.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE projects SET head=1 WHERE id=?")
            .bind(&project)
            .execute(&s.pool)
            .await
            .unwrap();
        let Json(result) = read(
            State(s.clone()),
            axum::Extension(who),
            Query(Options {
                after: Some(format!("vault-v2:{project}:delta:0")),
                limit: Some(1),
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["items"], json!([]));
        assert_eq!(result["next_cursor"], format!("vault-v2:{project}:delta:1"));
        assert_eq!(result["has_more"], false);
        let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM events WHERE project_id=?")
            .bind(&project)
            .fetch_one(&s.pool)
            .await
            .unwrap()
            .try_get("n")
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn stale_identity_cannot_read_feed() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::connect(&format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("feed.db").display()
        ))
        .await
        .unwrap();
        s.migrate().await.unwrap();
        let (project, token) = s.provision("feed", "reader").await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let who = s.authenticate(&headers).await.unwrap();
        s.revoke_token(&project, &who.token_id).await.unwrap();
        assert!(matches!(
            read(
                State(s),
                axum::Extension(who),
                Query(Options {
                    after: None,
                    limit: None
                })
            )
            .await,
            Err(Error(StatusCode::UNAUTHORIZED, "unauthorized"))
        ));
    }
}
