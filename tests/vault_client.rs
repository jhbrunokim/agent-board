use agent_board::{Store, router, vault};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{fs, path::Path};
use tower::ServiceExt;

async fn request(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    input: Value,
) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("content-type", "application/json")
                .body(Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn serve(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, server)
}
async fn setup(temp: &Path) -> (Store, String, String, axum::Router) {
    let store = Store::connect(&format!(
        "sqlite://{}?mode=rwc",
        temp.join("board.db").display()
    ))
    .await
    .unwrap();
    store.migrate().await.unwrap();
    let (project, token) = store.provision("vault", "owner").await.unwrap();
    let app = router(store.clone());
    (store, project, token, app)
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[tokio::test]
async fn actual_router_incremental_markdown_and_conflict_cursor() {
    let temp = tempfile::tempdir().unwrap();
    let (store, project, token, app) = setup(temp.path()).await;
    store
        .grant_role(&project, "reviewer", agent_board::Role::Reviewer)
        .await
        .unwrap();
    let knowledge = request(&app, "POST", "/api/v1/knowledge", &token, "knowledge", json!({"title":"Evidence [[untrusted]] <b> ![embed](bad)","body":"Evidence body\n![[private]]\n<script>alert(1)</script>"})).await;
    let kid = knowledge["data"]["id"].as_str().unwrap();
    let kp = format!("/api/v1/knowledge/{kid}");
    let source = request(
        &app,
        "POST",
        "/api/v1/sources",
        &token,
        "source",
        json!({"label":"Reference","uri":"file:///private/no-fetch","revision":"commit-1"}),
    )
    .await;
    let sid = source["data"]["id"].as_str().unwrap();
    request(
        &app,
        "POST",
        &format!("{kp}/sources"),
        &token,
        "attach",
        json!({"expected_version":1,"expected_content_revision":1,"source_id":sid}),
    )
    .await;
    let task = request(
        &app,
        "POST",
        "/api/v1/tasks",
        &token,
        "task",
        json!({"title":"Ship report","description":"Acceptance criteria"}),
    )
    .await;
    let tid = task["data"]["id"].as_str().unwrap();
    let tp = format!("/api/v1/tasks/{tid}");
    request(
        &app,
        "POST",
        &format!("{tp}/claims"),
        &token,
        "claim",
        json!({"expected_version":1,"expected_epoch":0,"run_id":"run","lease_seconds":3600}),
    )
    .await;
    request(&app, "POST", &format!("{tp}/knowledge"), &token, "link", json!({"expected_task_version":2,"run_id":"run","claim_epoch":1,"knowledge_id":kid,"knowledge_content_revision":2})).await;
    let review = request(&app, "POST", "/api/v1/reviews", &token, "review", json!({"subject_kind":"task","subject_id":tid,"subject_revision":1,"expected_version":3,"reviewer":"reviewer"})).await;
    let rid = review["data"]["id"].as_str().unwrap();
    let thread = request(
        &app,
        "POST",
        "/api/v1/threads",
        &token,
        "thread",
        json!({"title":"Discussion","payload":{"text":"First reply"}}),
    )
    .await;
    let thread_id = thread["thread_id"].as_str().unwrap();
    request(
        &app,
        "POST",
        &format!("/api/v1/threads/{thread_id}/messages"),
        &token,
        "reply",
        json!({"title":null,"payload":{"nested":{"number":42},"text":"Second reply"}}),
    )
    .await;
    let (url, server) = serve(app.clone()).await;
    let local = temp.path().join("vault");
    fs::create_dir(&local).unwrap();
    fs::create_dir(local.join("PersonalNotes")).unwrap();
    fs::write(local.join("PersonalNotes/mine.md"), "personal").unwrap();
    fs::create_dir(local.join(".obsidian")).unwrap();
    fs::write(local.join(".obsidian/config.json"), "settings").unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 5);
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 0);
    let dir = local.join("AgentBoard").join(&project);
    let knowledge_note = dir.join(format!("object-{kid}.md"));
    let note = fs::read_to_string(&knowledge_note).unwrap();
    assert!(note.contains("## Body\n"));
    assert!(note.contains("    ![[private]]\n"));
    assert!(note.contains("&lt;script&gt;"));
    assert!(!note.contains("# Evidence [[untrusted]]"));
    assert!(note.contains(&format!("[[AgentBoard/{project}/source-{sid}]]")));
    let task_note = fs::read_to_string(dir.join(format!("object-{tid}.md"))).unwrap();
    assert!(task_note.contains("## Lease"));
    assert!(task_note.contains("Acceptance criteria"));
    assert!(task_note.contains(&format!("[[AgentBoard/{project}/object-{kid}]]")));
    assert!(task_note.contains("Pinned content revision"));
    let thread_note = fs::read_to_string(dir.join(format!("thread-{thread_id}.md"))).unwrap();
    assert!(thread_note.find("First reply").unwrap() < thread_note.find("Second reply").unwrap());
    assert!(thread_note.contains("### owner — "));
    assert!(thread_note.contains("\"number\": 42"));
    assert!(
        fs::read_to_string(dir.join(format!("source-{sid}.md")))
            .unwrap()
            .contains("    file:///private/no-fetch")
    );
    let review_note = fs::read_to_string(dir.join(format!("review-{rid}.md"))).unwrap();
    assert!(review_note.contains("Reviewer"));
    assert!(review_note.contains(&format!("[[AgentBoard/{project}/object-{tid}]]")));
    let task_before = fs::read(dir.join(format!("object-{tid}.md"))).unwrap();
    request(
        &app,
        "POST",
        &format!("{kp}/revisions"),
        &token,
        "revision",
        json!({"expected_version":2,"title":"Updated evidence","body":"Updated body"}),
    )
    .await;
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 1);
    assert_eq!(
        fs::read(dir.join(format!("object-{tid}.md"))).unwrap(),
        task_before
    );
    assert!(
        fs::read_to_string(&knowledge_note)
            .unwrap()
            .contains("Updated body")
    );
    let state_path = dir.join("sync-state.json");
    let committed = fs::read(&state_path).unwrap();
    fs::write(&knowledge_note, "my edits").unwrap();
    assert!(
        vault::pull(&url, &token, &local)
            .await
            .unwrap_err()
            .to_string()
            .contains("user edit conflict")
    );
    assert_eq!(fs::read(&state_path).unwrap(), committed);
    assert_eq!(fs::read_to_string(&knowledge_note).unwrap(), "my edits");
    assert_eq!(
        fs::read_to_string(local.join("PersonalNotes/mine.md")).unwrap(),
        "personal"
    );
    assert_eq!(
        fs::read_to_string(local.join(".obsidian/config.json")).unwrap(),
        "settings"
    );
    server.abort();
}

#[tokio::test]
async fn journal_recovers_written_note_and_url_change_bootstraps() {
    let temp = tempfile::tempdir().unwrap();
    let (_, project, token, app) = setup(temp.path()).await;
    let created = request(
        &app,
        "POST",
        "/api/v1/knowledge",
        &token,
        "create",
        json!({"title":"Old","body":"Old"}),
    )
    .await;
    let id = created["data"]["id"].as_str().unwrap();
    let (url, server) = serve(app.clone()).await;
    let local = temp.path().join("vault");
    fs::create_dir(&local).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 1);
    let dir = local.join("AgentBoard").join(project);
    let mp = dir.join("manifest.json");
    let sp = dir.join("sync-state.json");
    let mut interrupted = read_json(&sp);
    let old_manifest = fs::read(&mp).unwrap();
    request(
        &app,
        "POST",
        &format!("/api/v1/knowledge/{id}/revisions"),
        &token,
        "revision",
        json!({"expected_version":1,"title":"New","body":"New"}),
    )
    .await;
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 1);
    let note = dir.join(format!("object-{id}.md"));
    interrupted["pending"] = json!({format!("object-{id}"):sha(&fs::read(note).unwrap())});
    // Simulate crash between note rename and manifest update.
    fs::write(&mp, old_manifest).unwrap();
    fs::write(&sp, interrupted.to_string()).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 0);
    assert_eq!(read_json(&sp)["pending"], json!({}));
    let mut old_server_state = read_json(&sp);
    old_server_state["server_url"] = json!("http://old.invalid");
    old_server_state["cursor"] = json!("invalid-on-this-server");
    fs::write(&sp, old_server_state.to_string()).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 0);
    assert_eq!(read_json(&sp)["server_url"], url);
    server.abort();
}

#[tokio::test]
async fn legacy_manifest_and_message_notes_survive_bootstrap() {
    let temp = tempfile::tempdir().unwrap();
    let (_, project, token, app) = setup(temp.path()).await;
    request(
        &app,
        "POST",
        "/api/v1/knowledge",
        &token,
        "create",
        json!({"title":"Note","body":"Body"}),
    )
    .await;
    let (url, server) = serve(app).await;
    let local = temp.path().join("vault");
    fs::create_dir(&local).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 1);
    let dir = local.join("AgentBoard").join(project);
    let mp = dir.join("manifest.json");
    let sp = dir.join("sync-state.json");
    let legacy_id = format!("message-{}", uuid::Uuid::new_v4());
    fs::write(dir.join(format!("{legacy_id}.md")), "Legacy message").unwrap();
    let mut manifest = read_json(&mp);
    manifest[&legacy_id] = json!(sha(b"Legacy message"));
    fs::write(mp, manifest.to_string()).unwrap();
    fs::remove_file(sp).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 0);
    assert_eq!(
        fs::read_to_string(dir.join(format!("{legacy_id}.md"))).unwrap(),
        "Legacy message"
    );
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_and_existing_export_lock_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let (_, project, token, app) = setup(temp.path()).await;
    let created = request(
        &app,
        "POST",
        "/api/v1/knowledge",
        &token,
        "create",
        json!({"title":"Note","body":"Body"}),
    )
    .await;
    let id = created["data"]["id"].as_str().unwrap();
    let (url, server) = serve(app).await;
    let local = temp.path().join("vault");
    fs::create_dir(&local).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, local.join("AgentBoard")).unwrap();
    assert!(vault::pull(&url, &token, &local).await.is_err());
    fs::remove_file(local.join("AgentBoard")).unwrap();
    assert_eq!(vault::pull(&url, &token, &local).await.unwrap(), 1);
    let root = local.join("AgentBoard");
    fs::write(root.join(".export.lock"), "occupied").unwrap();
    assert!(vault::pull(&url, &token, &local).await.is_err());
    fs::remove_file(root.join(".export.lock")).unwrap();
    let note = root.join(project).join(format!("object-{id}.md"));
    fs::remove_file(&note).unwrap();
    let target = outside.join("personal.md");
    fs::write(&target, "personal").unwrap();
    symlink(&target, note).unwrap();
    assert!(vault::pull(&url, &token, &local).await.is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "personal");
    server.abort();
}
