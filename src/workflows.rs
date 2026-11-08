//! Project-scoped tasks, leases, independent reviews, and immutable knowledge revisions.
use super::*;
use axum::routing::patch;
use std::time::{SystemTime, UNIX_EPOCH};

type Tx<'a> = sqlx::Transaction<'a, sqlx::Any>;

pub(super) async fn migrate(tx: &mut Tx<'_>) -> std::result::Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS wf_objects (project_id TEXT NOT NULL REFERENCES projects(id), id TEXT NOT NULL, kind TEXT NOT NULL, version BIGINT NOT NULL, state TEXT NOT NULL, PRIMARY KEY(project_id,id))",
        "CREATE TABLE IF NOT EXISTS wf_object_history (project_id TEXT NOT NULL, id TEXT NOT NULL, version BIGINT NOT NULL, state TEXT NOT NULL, PRIMARY KEY(project_id,id,version), FOREIGN KEY(project_id,id) REFERENCES wf_objects(project_id,id))",
        "CREATE TABLE IF NOT EXISTS wf_reviews (project_id TEXT NOT NULL REFERENCES projects(id), id TEXT NOT NULL, version BIGINT NOT NULL, subject_id TEXT NOT NULL, subject_kind TEXT NOT NULL, subject_revision BIGINT NOT NULL, state TEXT NOT NULL, PRIMARY KEY(project_id,id), FOREIGN KEY(project_id,subject_id) REFERENCES wf_objects(project_id,id))",
        "CREATE TABLE IF NOT EXISTS wf_review_history (project_id TEXT NOT NULL, id TEXT NOT NULL, version BIGINT NOT NULL, state TEXT NOT NULL, PRIMARY KEY(project_id,id,version), FOREIGN KEY(project_id,id) REFERENCES wf_reviews(project_id,id))",
        "CREATE INDEX IF NOT EXISTS wf_reviews_subject ON wf_reviews(project_id,subject_id,subject_revision)",
    ] {
        sqlx::query(sql).execute(&mut **tx).await?;
    }
    Ok(())
}

pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/tasks", post(create_task))
        .route(
            "/api/v1/tasks/{id}",
            get(get_task).merge(patch(update_task)),
        )
        .route("/api/v1/tasks/{id}/claims", post(claim_task))
        .route("/api/v1/knowledge", post(create_knowledge))
        .route("/api/v1/knowledge/{id}", get(get_knowledge))
        .route("/api/v1/knowledge/{id}/revisions", post(revise_knowledge))
        .route("/api/v1/reviews", post(create_review))
        .route("/api/v1/reviews/{id}", get(get_review))
        .route("/api/v1/reviews/{id}/verdicts", post(verdict))
}

fn state_conflict(code: &'static str) -> Error {
    Error(StatusCode::CONFLICT, code)
}
fn forbidden() -> Error {
    Error(StatusCode::FORBIDDEN, "forbidden")
}
fn bounded(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max
}
fn now_ms() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .ok_or(Error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clock_unavailable",
        ))
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_stored_data"))
}
fn value<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v)
        .map_err(|_| Error(StatusCode::INTERNAL_SERVER_ERROR, "serialization_failed"))
}
fn version(actual: i64, expected: i64) -> Result<()> {
    if expected < 1 {
        Err(invalid())
    } else if actual != expected {
        Err(state_conflict("version_conflict"))
    } else {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Lease {
    principal: String,
    run_id: String,
    epoch: i64,
    expires_at_ms: i64,
}
#[derive(Serialize, Deserialize)]
struct Task {
    id: String,
    project_id: String,
    version: i64,
    content_revision: i64,
    title: String,
    description: String,
    status: String,
    created_by: String,
    content_author: String,
    lease_epoch: i64,
    lease: Option<Lease>,
}
impl Task {
    fn require_lease(&self, who: &Identity, run: &str, epoch: i64, now: i64) -> Result<()> {
        if !bounded(run, 128) || epoch < 1 {
            return Err(invalid());
        }
        let lease = self
            .lease
            .as_ref()
            .ok_or(state_conflict("claim_required"))?;
        if lease.epoch != epoch || self.lease_epoch != epoch {
            return Err(state_conflict("stale_claim"));
        }
        if lease.principal != who.principal || lease.run_id != run {
            return Err(forbidden());
        }
        if lease.expires_at_ms <= now {
            return Err(state_conflict("claim_expired"));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Knowledge {
    id: String,
    project_id: String,
    version: i64,
    content_revision: i64,
    title: String,
    body: String,
    status: String,
    created_by: String,
    content_author: String,
}
#[derive(Serialize, Deserialize)]
struct Review {
    id: String,
    project_id: String,
    version: i64,
    subject_kind: String,
    subject_id: String,
    subject_revision: i64,
    requester: String,
    reviewer: String,
    content_author: String,
    status: String,
    comment: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTask {
    title: String,
    description: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    expected_version: i64,
    expected_epoch: i64,
    run_id: String,
    lease_seconds: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskUpdate {
    expected_version: i64,
    run_id: String,
    claim_epoch: i64,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewKnowledge {
    title: String,
    body: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_version: i64,
    title: String,
    body: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewReview {
    subject_kind: String,
    subject_id: String,
    subject_revision: i64,
    expected_version: i64,
    reviewer: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Verdict {
    expected_version: i64,
    verdict: String,
    comment: Option<String>,
}

enum Command {
    NewTask(NewTask),
    Claim(String, Claim),
    TaskUpdate(String, TaskUpdate),
    NewKnowledge(NewKnowledge),
    Revision(String, Revision),
    NewReview(NewReview),
    Verdict(String, Verdict),
}
impl Command {
    fn operation(&self) -> String {
        match self {
            Self::NewTask(_) => "wf:task:create".into(),
            Self::Claim(id, _) => format!("wf:task:claim:{id}"),
            Self::TaskUpdate(id, _) => format!("wf:task:update:{id}"),
            Self::NewKnowledge(_) => "wf:knowledge:create".into(),
            Self::Revision(id, _) => format!("wf:knowledge:revision:{id}"),
            Self::NewReview(_) => "wf:review:create".into(),
            Self::Verdict(id, _) => format!("wf:review:verdict:{id}"),
        }
    }
    fn input(&self) -> Result<Value> {
        match self {
            Self::NewTask(v) => value(v),
            Self::Claim(_, v) => value(v),
            Self::TaskUpdate(_, v) => value(v),
            Self::NewKnowledge(v) => value(v),
            Self::Revision(_, v) => value(v),
            Self::NewReview(v) => value(v),
            Self::Verdict(_, v) => value(v),
        }
    }
}

async fn load<T: serde::de::DeserializeOwned>(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    kind: &str,
) -> Result<T> {
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
async fn save_object<T: Serialize>(
    s: &Store,
    tx: &mut Tx<'_>,
    who: &Identity,
    id: &str,
    kind: &str,
    v: i64,
    object: &T,
) -> Result<()> {
    let state = value(object)?.to_string();
    sqlx::query(&s.sql("INSERT INTO wf_objects(project_id,id,kind,version,state) VALUES (?,?,?,?,?) ON CONFLICT(project_id,id) DO UPDATE SET version=excluded.version,state=excluded.state"))
        .bind(&who.project).bind(id).bind(kind).bind(v).bind(&state).execute(&mut **tx).await?;
    sqlx::query(
        &s.sql("INSERT INTO wf_object_history(project_id,id,version,state) VALUES (?,?,?,?)"),
    )
    .bind(&who.project)
    .bind(id)
    .bind(v)
    .bind(state)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
async fn save_review(s: &Store, tx: &mut Tx<'_>, who: &Identity, review: &Review) -> Result<()> {
    let state = value(review)?.to_string();
    sqlx::query(&s.sql("INSERT INTO wf_reviews(project_id,id,version,subject_id,subject_kind,subject_revision,state) VALUES (?,?,?,?,?,?,?) ON CONFLICT(project_id,id) DO UPDATE SET version=excluded.version,state=excluded.state"))
        .bind(&who.project).bind(&review.id).bind(review.version).bind(&review.subject_id).bind(&review.subject_kind).bind(review.subject_revision).bind(&state).execute(&mut **tx).await?;
    sqlx::query(
        &s.sql("INSERT INTO wf_review_history(project_id,id,version,state) VALUES (?,?,?,?)"),
    )
    .bind(&who.project)
    .bind(&review.id)
    .bind(review.version)
    .bind(state)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn approved(s: &Store, tx: &mut Tx<'_>, who: &Identity, task: &Task) -> Result<bool> {
    let rows = sqlx::query(&s.sql("SELECT state FROM wf_reviews WHERE project_id=? AND subject_id=? AND subject_kind='task' AND subject_revision=?"))
        .bind(&who.project).bind(&task.id).bind(task.content_revision).fetch_all(&mut **tx).await?;
    for row in rows {
        let review: Review = decode(&row.try_get::<String, _>("state")?)?;
        if review.status == "approved"
            && review.reviewer != who.principal
            && review.reviewer != task.content_author
            && review.reviewer != task.created_by
            && review.reviewer != review.requester
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn mutate(s: &Store, who: &Identity, key: &str, command: Command) -> Result<Value> {
    if !bounded(key, 128) {
        return Err(invalid());
    }
    let operation = command.operation();
    let digest = hash(&command.input()?.to_string());
    let mut tx = s.pool.begin().await?;
    let seq: i64 = sqlx::query(&s.sql("UPDATE projects SET head=head+1 WHERE id=? RETURNING head"))
        .bind(&who.project)
        .fetch_one(&mut *tx)
        .await?
        .try_get("head")?;
    s.active(who, &mut tx).await?;
    let previous = sqlx::query(&s.sql("SELECT hash,response FROM dedup WHERE project_id=? AND principal=? AND operation=? AND key=?"))
        .bind(&who.project).bind(&who.principal).bind(&operation).bind(key).fetch_optional(&mut *tx).await?;
    if let Some(row) = previous {
        tx.rollback().await?;
        if row.try_get::<String, _>("hash")? != digest {
            return Err(conflict());
        }
        return decode(&row.try_get::<String, _>("response")?);
    }
    // Read the clock after acquiring the project lock: time spent waiting cannot extend a lease.
    let now = now_ms()?;
    let response = match command {
        Command::NewTask(input) => {
            if !bounded(&input.title, 500) || !bounded(&input.description, 65536) {
                return Err(invalid());
            }
            let task = Task {
                id: Uuid::new_v4().to_string(),
                project_id: who.project.clone(),
                version: 1,
                content_revision: 1,
                title: input.title,
                description: input.description,
                status: "open".into(),
                created_by: who.principal.clone(),
                content_author: who.principal.clone(),
                lease_epoch: 0,
                lease: None,
            };
            save_object(s, &mut tx, who, &task.id, "task", task.version, &task).await?;
            value(&task)?
        }
        Command::Claim(id, input) => {
            if !bounded(&input.run_id, 128)
                || !(1..=3600).contains(&input.lease_seconds)
                || input.expected_epoch < 0
            {
                return Err(invalid());
            }
            let mut task: Task = load(s, &mut tx, who, &id, "task").await?;
            version(task.version, input.expected_version)?;
            if task.status == "done" {
                return Err(state_conflict("task_done"));
            }
            if task.lease_epoch != input.expected_epoch {
                return Err(state_conflict("stale_claim"));
            }
            if task.lease.as_ref().is_some_and(|l| l.expires_at_ms > now) {
                task.require_lease(who, &input.run_id, input.expected_epoch, now)?;
            } else {
                task.lease_epoch += 1;
            }
            task.lease = Some(Lease {
                principal: who.principal.clone(),
                run_id: input.run_id,
                epoch: task.lease_epoch,
                expires_at_ms: now + input.lease_seconds * 1000,
            });
            task.status = "in_progress".into();
            task.version += 1;
            save_object(s, &mut tx, who, &id, "task", task.version, &task).await?;
            value(&task)?
        }
        Command::TaskUpdate(id, input) => {
            if input.title.as_ref().is_some_and(|v| !bounded(v, 500))
                || input
                    .description
                    .as_ref()
                    .is_some_and(|v| !bounded(v, 65536))
                || input.status.as_ref().is_some_and(|v| {
                    !["open", "in_progress", "in_review", "blocked", "done"].contains(&v.as_str())
                })
                || (input.title.is_none() && input.description.is_none() && input.status.is_none())
            {
                return Err(invalid());
            }
            let mut task: Task = load(s, &mut tx, who, &id, "task").await?;
            version(task.version, input.expected_version)?;
            task.require_lease(who, &input.run_id, input.claim_epoch, now)?;
            if task.status == "done" {
                return Err(state_conflict("task_done"));
            }
            let changed = input.title.as_ref().is_some_and(|v| v != &task.title)
                || input
                    .description
                    .as_ref()
                    .is_some_and(|v| v != &task.description);
            if let Some(title) = input.title {
                task.title = title;
            }
            if let Some(description) = input.description {
                task.description = description;
            }
            if changed {
                task.content_revision += 1;
                task.content_author = who.principal.clone();
            }
            if input.status.as_deref() == Some("done") && !approved(s, &mut tx, who, &task).await? {
                return Err(state_conflict("approval_required"));
            }
            if let Some(status) = input.status {
                task.status = status;
            }
            if task.status == "done" || task.status == "open" {
                task.lease = None;
            }
            task.version += 1;
            save_object(s, &mut tx, who, &id, "task", task.version, &task).await?;
            value(&task)?
        }
        Command::NewKnowledge(input) => {
            if !bounded(&input.title, 500) || !bounded(&input.body, 65536) {
                return Err(invalid());
            }
            let knowledge = Knowledge {
                id: Uuid::new_v4().to_string(),
                project_id: who.project.clone(),
                version: 1,
                content_revision: 1,
                title: input.title,
                body: input.body,
                status: "draft".into(),
                created_by: who.principal.clone(),
                content_author: who.principal.clone(),
            };
            save_object(
                s,
                &mut tx,
                who,
                &knowledge.id,
                "knowledge",
                knowledge.version,
                &knowledge,
            )
            .await?;
            value(&knowledge)?
        }
        Command::Revision(id, input) => {
            if !bounded(&input.title, 500) || !bounded(&input.body, 65536) {
                return Err(invalid());
            }
            let mut knowledge: Knowledge = load(s, &mut tx, who, &id, "knowledge").await?;
            version(knowledge.version, input.expected_version)?;
            knowledge.version += 1;
            knowledge.content_revision += 1;
            knowledge.title = input.title;
            knowledge.body = input.body;
            knowledge.status = "draft".into();
            knowledge.content_author = who.principal.clone();
            save_object(
                s,
                &mut tx,
                who,
                &id,
                "knowledge",
                knowledge.version,
                &knowledge,
            )
            .await?;
            value(&knowledge)?
        }
        Command::NewReview(input) => {
            if !bounded(&input.reviewer, 256)
                || !bounded(&input.subject_id, 128)
                || input.subject_revision < 1
            {
                return Err(invalid());
            }
            let (subject_version, revision, author, creator, claimant) =
                match input.subject_kind.as_str() {
                    "task" => {
                        let task: Task = load(s, &mut tx, who, &input.subject_id, "task").await?;
                        (
                            task.version,
                            task.content_revision,
                            task.content_author,
                            task.created_by,
                            task.lease.map(|l| l.principal),
                        )
                    }
                    "knowledge" => {
                        let knowledge: Knowledge =
                            load(s, &mut tx, who, &input.subject_id, "knowledge").await?;
                        (
                            knowledge.version,
                            knowledge.content_revision,
                            knowledge.content_author,
                            knowledge.created_by,
                            None,
                        )
                    }
                    _ => return Err(invalid()),
                };
            version(subject_version, input.expected_version)?;
            if revision != input.subject_revision {
                return Err(state_conflict("stale_review"));
            }
            if input.reviewer == who.principal
                || input.reviewer == author
                || input.reviewer == creator
                || claimant.as_ref() == Some(&input.reviewer)
            {
                return Err(state_conflict("independent_reviewer_required"));
            }
            if sqlx::query(
                &s.sql("SELECT principal FROM tokens WHERE project_id=? AND principal=? AND revoked=0 AND role IN ('reviewer','admin') LIMIT 1"),
            )
            .bind(&who.project)
            .bind(&input.reviewer)
            .fetch_optional(&mut *tx)
            .await?
            .is_none()
            {
                return Err(invalid());
            }
            let review = Review {
                id: Uuid::new_v4().to_string(),
                project_id: who.project.clone(),
                version: 1,
                subject_kind: input.subject_kind,
                subject_id: input.subject_id,
                subject_revision: input.subject_revision,
                requester: who.principal.clone(),
                reviewer: input.reviewer,
                content_author: author,
                status: "pending".into(),
                comment: None,
            };
            save_review(s, &mut tx, who, &review).await?;
            value(&review)?
        }
        Command::Verdict(id, input) => {
            if !["approved", "rejected"].contains(&input.verdict.as_str())
                || input.comment.as_ref().is_some_and(|v| v.len() > 4096)
            {
                return Err(invalid());
            }
            let row =
                sqlx::query(&s.sql("SELECT state FROM wf_reviews WHERE project_id=? AND id=?"))
                    .bind(&who.project)
                    .bind(&id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(missing)?;
            let mut review: Review = decode(&row.try_get::<String, _>("state")?)?;
            version(review.version, input.expected_version)?;
            if review.reviewer != who.principal {
                return Err(forbidden());
            }
            if review.status != "pending" {
                return Err(state_conflict("review_closed"));
            }
            let subject = match review.subject_kind.as_str() {
                "task" => {
                    let task: Task = load(s, &mut tx, who, &review.subject_id, "task").await?;
                    if task.content_revision != review.subject_revision {
                        return Err(state_conflict("stale_review"));
                    }
                    if task.content_author == who.principal
                        || task.created_by == who.principal
                        || task
                            .lease
                            .as_ref()
                            .is_some_and(|l| l.principal == who.principal)
                    {
                        return Err(state_conflict("independent_reviewer_required"));
                    }
                    value(&task)?
                }
                "knowledge" => {
                    let mut knowledge: Knowledge =
                        load(s, &mut tx, who, &review.subject_id, "knowledge").await?;
                    if knowledge.content_revision != review.subject_revision {
                        return Err(state_conflict("stale_review"));
                    }
                    if knowledge.content_author == who.principal
                        || knowledge.created_by == who.principal
                    {
                        return Err(state_conflict("independent_reviewer_required"));
                    }
                    knowledge.status = if input.verdict == "approved" {
                        "verified"
                    } else {
                        "draft"
                    }
                    .into();
                    knowledge.version += 1;
                    save_object(
                        s,
                        &mut tx,
                        who,
                        &knowledge.id,
                        "knowledge",
                        knowledge.version,
                        &knowledge,
                    )
                    .await?;
                    value(&knowledge)?
                }
                _ => return Err(invalid()),
            };
            if review.requester == who.principal || review.content_author == who.principal {
                return Err(state_conflict("independent_reviewer_required"));
            }
            review.status = input.verdict;
            review.comment = input.comment;
            review.version += 1;
            save_review(s, &mut tx, who, &review).await?;
            json!({"review":review,"subject":subject})
        }
    };
    let response = json!({"data":response,"sequence":seq});
    let event = json!({"type":operation,"actor":who.principal,"entity_id":response["data"]["id"],"review_id":response["data"]["review"]["id"],"subject_id":response["data"]["subject"]["id"],"sequence":seq});
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadVersion {
    version: Option<i64>,
}
async fn read_object(
    s: Store,
    who: Identity,
    id: String,
    kind: &str,
    page: ReadVersion,
) -> Result<Json<Value>> {
    if page.version.is_some_and(|v| v < 1) {
        return Err(invalid());
    }
    let row = if let Some(v) = page.version {
        sqlx::query(&s.sql("SELECT h.state FROM wf_object_history h JOIN wf_objects o ON h.project_id=o.project_id AND h.id=o.id WHERE h.project_id=? AND h.id=? AND o.kind=? AND h.version=?"))
            .bind(&who.project).bind(id).bind(kind).bind(v).fetch_optional(&s.pool).await?
    } else {
        sqlx::query(&s.sql("SELECT state FROM wf_objects WHERE project_id=? AND id=? AND kind=?"))
            .bind(&who.project).bind(id).bind(kind).fetch_optional(&s.pool).await?
    }.ok_or_else(missing)?;
    Ok(Json(
        json!({"data":decode::<Value>(&row.try_get::<String, _>("state")?)?}),
    ))
}
async fn get_task(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    Query(page): Query<ReadVersion>,
) -> Result<Json<Value>> {
    read_object(s, who, id, "task", page).await
}
async fn get_knowledge(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
    Query(page): Query<ReadVersion>,
) -> Result<Json<Value>> {
    read_object(s, who, id, "knowledge", page).await
}
macro_rules! create_handler {
    ($name:ident, $input:ty, $variant:ident) => {
        async fn $name(
            State(s): State<Store>,
            axum::Extension(who): axum::Extension<Identity>,
            h: HeaderMap,
            Json(input): Json<$input>,
        ) -> Result<Json<Value>> {
            Ok(Json(
                mutate(&s, &who, key(&h)?, Command::$variant(input)).await?,
            ))
        }
    };
}
macro_rules! object_handler {
    ($name:ident, $input:ty, $variant:ident) => {
        async fn $name(
            State(s): State<Store>,
            axum::Extension(who): axum::Extension<Identity>,
            Path(id): Path<String>,
            h: HeaderMap,
            Json(input): Json<$input>,
        ) -> Result<Json<Value>> {
            Ok(Json(
                mutate(&s, &who, key(&h)?, Command::$variant(id, input)).await?,
            ))
        }
    };
}
create_handler!(create_task, NewTask, NewTask);
create_handler!(create_knowledge, NewKnowledge, NewKnowledge);
create_handler!(create_review, NewReview, NewReview);
object_handler!(claim_task, Claim, Claim);
object_handler!(update_task, TaskUpdate, TaskUpdate);
object_handler!(revise_knowledge, Revision, Revision);
object_handler!(verdict, Verdict, Verdict);

async fn get_review(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let row = sqlx::query(&s.sql("SELECT state FROM wf_reviews WHERE project_id=? AND id=?"))
        .bind(&who.project)
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(missing)?;
    Ok(Json(
        json!({"data":decode::<Value>(&row.try_get::<String,_>("state")?)?}),
    ))
}
