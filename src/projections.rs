//! Bounded, project-scoped read models shared by agent queries and vault export.
use super::*;

pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/search", get(search))
        .route("/api/v1/context", get(context))
        .route("/api/v1/export", get(export))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    q: Option<String>,
    after: Option<String>,
    limit: Option<i64>,
    max_chars: Option<usize>,
}
async fn documents(s: &Store, who: &Identity, o: &Options) -> Result<(Vec<Value>, bool)> {
    let limit = o.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) || o.q.as_ref().is_some_and(|q| q.len() > 256) {
        return Err(invalid());
    }
    let query = o.q.as_deref().unwrap_or("");
    let scope = hash(&format!("{}:{query}", who.project));
    let after = if let Some(c) = &o.after {
        let (prefix, key) = c.split_once(':').ok_or_else(invalid)?;
        if prefix != scope {
            return Err(invalid());
        }
        key
    } else {
        ""
    };
    let contains = if s.postgres {
        "strpos(lower(body), lower(?)) > 0"
    } else {
        "instr(lower(body), lower(?)) > 0"
    };
    // No JSON extraction dependency: original JSON is the searchable portable body.
    let sql = format!(
        "SELECT doc_key,body FROM (SELECT 'object-' || id AS doc_key,state AS body FROM wf_objects WHERE project_id=? UNION ALL SELECT 'message-' || id AS doc_key,payload AS body FROM messages WHERE project_id=?) d WHERE doc_key>? AND {contains} ORDER BY doc_key LIMIT ?"
    );
    let rows = sqlx::query(&s.sql(&sql))
        .bind(&who.project)
        .bind(&who.project)
        .bind(after)
        .bind(query)
        .bind(limit + 1)
        .fetch_all(&s.pool)
        .await?;
    let more = rows.len() > limit as usize;
    let mut out = Vec::new();
    for row in rows.into_iter().take(limit as usize) {
        let key: String = row.try_get("doc_key")?;
        let body: Value =
            serde_json::from_str(&row.try_get::<String, _>("body")?).map_err(|_| invalid())?;
        out.push(json!({"id":key,"project_id":who.project,"data":body,"cursor":format!("{scope}:{key}")}));
    }
    Ok((out, more))
}
async fn export(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(o): Query<Options>,
) -> Result<Json<Value>> {
    let (items, more) = documents(&s, &who, &o).await?;
    let cursor = items
        .last()
        .map(|v| v["cursor"].clone())
        .unwrap_or(Value::Null);
    Ok(Json(
        json!({"project_id":who.project,"items":items,"has_more":more,"next_cursor":cursor}),
    ))
}
async fn search(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(o): Query<Options>,
) -> Result<Json<Value>> {
    let (items, more) = documents(&s, &who, &o).await?;
    let cursor = items
        .last()
        .map(|v| v["cursor"].clone())
        .unwrap_or(Value::Null);
    let summaries:Vec<Value>=items.iter().map(|v|json!({"id":v["id"],"title":v["data"]["title"],"status":v["data"]["status"],"version":v["data"]["version"],"excerpt":v["data"].to_string().chars().take(300).collect::<String>()})).collect();
    Ok(Json(
        json!({"items":summaries,"next_cursor":cursor,"has_more":more}),
    ))
}
async fn context(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Query(o): Query<Options>,
) -> Result<Json<Value>> {
    let budget = o.max_chars.unwrap_or(8000);
    if !(512..=32000).contains(&budget) {
        return Err(invalid());
    }
    let (items, more) = documents(&s, &who, &o).await?;
    let mut selected = Vec::new();
    let mut omitted = 0;
    let mut used = 0;
    let mut last = Value::Null;
    for item in items {
        let n = item.to_string().chars().count();
        if used + n <= budget {
            used += n;
            last = item["cursor"].clone();
            selected.push(item);
        } else {
            omitted += 1;
            break;
        }
    }
    // next_cursor only covers returned objects; oversized first item must be fetched via export.
    Ok(Json(
        json!({"items":selected,"content_chars":used,"omitted_at_least":omitted,"incomplete":more||omitted>0,"next_cursor":last,"max_chars":budget,"lookup":"/api/v1/export"}),
    ))
}
