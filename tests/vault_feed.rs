use agent_board::{Role, Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;
async fn request(
    app: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn ok(
    app: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    body: Value,
) -> Value {
    let (status, body) = request(app, method, path, token, key, body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn page(app: axum::Router, token: &str, cursor: Option<&str>, limit: usize) -> Value {
    let path = match cursor {
        Some(cursor) => format!("/api/v1/vault?limit={limit}&after={cursor}"),
        None => format!("/api/v1/vault?limit={limit}"),
    };
    ok(app, "GET", &path, token, "", Value::Null).await
}
fn cursor(value: &Value) -> &str {
    value["next_cursor"].as_str().unwrap()
}
async fn contract(url: &str) {
    let s = Store::connect(url).await.unwrap();
    s.migrate().await.unwrap();
    let (project, token) = s.provision("feed", "owner").await.unwrap();
    let reader = s
        .grant_role(&project, "reader", Role::Reader)
        .await
        .unwrap();
    let _reviewer = s
        .grant_role(&project, "reviewer", Role::Reviewer)
        .await
        .unwrap();
    let (_, outsider) = s.provision("other", "other").await.unwrap();
    let app = router(s.clone());
    let knowledge = ok(
        app.clone(),
        "POST",
        "/api/v1/knowledge",
        &token,
        "knowledge",
        json!({"title":"Note","body":"original"}),
    )
    .await;
    let kid = knowledge["data"]["id"].as_str().unwrap();
    let task = ok(
        app.clone(),
        "POST",
        "/api/v1/tasks",
        &token,
        "task",
        json!({"title":"Work","description":"Description"}),
    )
    .await;
    let tid = task["data"]["id"].as_str().unwrap();
    let source = ok(
        app.clone(),
        "POST",
        "/api/v1/sources",
        &token,
        "source",
        json!({"uri":"http://127.0.0.1:1/no-fetch","revision":"commit:123","label":"Evidence"}),
    )
    .await;
    let sid = source["data"]["id"].as_str().unwrap();
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/knowledge/{kid}/sources"),
        &token,
        "attach",
        json!({"expected_version":1,"expected_content_revision":1,"source_id":sid}),
    )
    .await;
    let review = ok(app.clone(),"POST","/api/v1/reviews",&token,"review",json!({"subject_kind":"knowledge","subject_id":kid,"subject_revision":2,"expected_version":2,"reviewer":"reviewer"})).await;
    let rid = review["data"]["id"].as_str().unwrap();
    let thread = ok(
        app.clone(),
        "POST",
        "/api/v1/threads",
        &token,
        "thread",
        json!({"title":"Discussion","payload":{"text":"first"}}),
    )
    .await;
    let thread_id = thread["thread_id"].as_str().unwrap();
    let first = page(app.clone(), &reader, None, 2).await;
    assert_eq!(first["renderer_version"], 2);
    assert_eq!(first["has_more"], true);
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert!(cursor(&first).contains(":bootstrap:6:"));
    // Writes between bootstrap pages are repaired by replay from captured H=6.
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/knowledge/{kid}/revisions"),
        &token,
        "revise",
        json!({"expected_version":2,"title":"Note","body":"changed after bootstrap"}),
    )
    .await;
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/threads/{thread_id}/messages"),
        &token,
        "reply",
        json!({"payload":{"text":"new reply"}}),
    )
    .await;
    let mut docs = BTreeMap::new();
    let mut p = first;
    loop {
        for doc in p["items"].as_array().unwrap() {
            assert!(
                docs.insert(doc["id"].as_str().unwrap().to_owned(), doc.clone())
                    .is_none()
            );
        }
        if cursor(&p).contains(":delta:") {
            break;
        }
        p = page(app.clone(), &reader, Some(cursor(&p)), 2).await;
    }
    assert_eq!(docs.len(), 5);
    assert_eq!(docs[&format!("review-{rid}")]["kind"], "review");
    assert_eq!(
        docs[&format!("source-{sid}")]["data"]["uri"],
        "http://127.0.0.1:1/no-fetch"
    );
    assert_eq!(cursor(&p), format!("vault-v2:{project}:delta:6:8"));
    let changed = page(app.clone(), &reader, Some(cursor(&p)), 1).await;
    assert_eq!(changed["has_more"], true);
    assert_eq!(changed["items"][0]["id"], format!("object-{kid}"));
    assert_eq!(
        changed["items"][0]["data"]["body"],
        "changed after bootstrap"
    );
    assert_eq!(changed["items"][0]["data"]["sources"][0]["id"], sid);
    let replied = page(app.clone(), &reader, Some(cursor(&changed)), 1).await;
    assert_eq!(replied["has_more"], false);
    assert_eq!(replied["items"][0]["id"], format!("thread-{thread_id}"));
    assert_eq!(
        replied["items"][0]["data"]["messages"][1]["payload"]["text"],
        "new reply"
    );
    let idle = page(app.clone(), &reader, Some(cursor(&replied)), 50).await;
    assert_eq!(idle["items"], json!([]));
    assert_eq!(cursor(&idle), cursor(&replied));
    // Read-only pages neither increment project head nor append events.
    let events_before = ok(
        app.clone(),
        "GET",
        "/api/v1/events?limit=200",
        &reader,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(events_before["items"].as_array().unwrap().len(), 8);
    assert_eq!(
        idle,
        page(app.clone(), &reader, Some(cursor(&idle)), 50).await
    );
    assert_eq!(
        events_before,
        ok(
            app.clone(),
            "GET",
            "/api/v1/events?limit=200",
            &reader,
            "",
            Value::Null
        )
        .await
    );
    let claim = ok(
        app.clone(),
        "POST",
        &format!("/api/v1/tasks/{tid}/claims"),
        &token,
        "claim",
        json!({"expected_version":1,"expected_epoch":0,"run_id":"run","lease_seconds":300}),
    )
    .await;
    assert_eq!(claim["data"]["version"], 2);
    ok(app.clone(),"POST",&format!("/api/v1/tasks/{tid}/knowledge"),&token,"link",json!({"expected_task_version":2,"run_id":"run","claim_epoch":1,"knowledge_id":kid,"knowledge_content_revision":3})).await;
    let linked = page(app.clone(), &reader, Some(cursor(&idle)), 50).await;
    assert_eq!(linked["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        linked["items"][0]["data"]["knowledge_links"][0]["knowledge_version"],
        3
    );
    assert_eq!(
        linked["items"][0]["data"]["knowledge_links"][0]["knowledge_id"],
        kid
    );
    for (token, after, code) in [
        (
            outsider.as_str(),
            cursor(&idle).to_owned(),
            "cursor_scope_mismatch",
        ),
        (
            reader.as_str(),
            cursor(&idle).replace("vault-v2:", "vault-v1:"),
            "cursor_renderer_mismatch",
        ),
        (
            reader.as_str(),
            format!("vault-v2:{project}:delta:9999999"),
            "invalid_input",
        ),
        (
            reader.as_str(),
            format!("vault-v2:{project}:delta:-1"),
            "invalid_input",
        ),
    ] {
        let (status, body) = request(
            app.clone(),
            "GET",
            &format!("/api/v1/vault?after={after}"),
            token,
            "",
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], code);
    }
    assert_eq!(
        request(
            app.clone(),
            "GET",
            "/api/v1/vault?limit=51",
            &reader,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            app.clone(),
            "GET",
            "/api/v1/vault",
            "invalid",
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // One scanned event can refresh multiple documents; document count may exceed limit.
    let mut attachments = Vec::new();
    for index in 0..26 {
        let k = ok(
            app.clone(),
            "POST",
            "/api/v1/knowledge",
            &token,
            &format!("many-k-{index}"),
            json!({"title":"Many","body":"Evidence"}),
        )
        .await;
        let source = ok(
            app.clone(),
            "POST",
            "/api/v1/sources",
            &token,
            &format!("many-s-{index}"),
            json!({"uri":"urn:test:source","revision":"1","label":"Evidence"}),
        )
        .await;
        attachments.push((
            k["data"]["id"].as_str().unwrap().to_owned(),
            source["data"]["id"].as_str().unwrap().to_owned(),
        ));
    }
    let mut baseline = page(app.clone(), &reader, None, 50).await;
    while baseline["has_more"] == true {
        baseline = page(app.clone(), &reader, Some(cursor(&baseline)), 50).await;
    }
    for (index, (kid, sid)) in attachments.iter().enumerate() {
        ok(
            app.clone(),
            "POST",
            &format!("/api/v1/knowledge/{kid}/sources"),
            &token,
            &format!("many-a-{index}"),
            json!({"expected_version":1,"expected_content_revision":1,"source_id":sid}),
        )
        .await;
    }
    let multi = page(app.clone(), &reader, Some(cursor(&baseline)), 50).await;
    assert_eq!(multi["items"].as_array().unwrap().len(), 52);
    assert_eq!(multi["has_more"], false);
    assert!(multi["content_chars"].as_u64().unwrap() <= 128_000);
    // A bounded catch-up keeps its watermark even while newer writes arrive.
    let mut catchup = page(app.clone(), &reader, Some(cursor(&baseline)), 1).await;
    assert_eq!(catchup["has_more"], true);
    let final_target = cursor(&multi).to_owned();
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/threads/{thread_id}/messages"),
        &token,
        "during-catchup",
        json!({"payload":{"text":"after bounded target"}}),
    )
    .await;
    while catchup["has_more"] == true {
        catchup = page(app.clone(), &reader, Some(cursor(&catchup)), 1).await;
    }
    assert_eq!(cursor(&catchup), final_target);
    let later = page(app.clone(), &reader, Some(cursor(&catchup)), 50).await;
    assert_eq!(later["items"].as_array().unwrap().len(), 1);
    assert_eq!(later["items"][0]["id"], format!("thread-{thread_id}"));
    s.check_schema().await.unwrap();
}
#[tokio::test]
async fn sqlite_vault_feed() {
    let d = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("feed.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_vault_feed() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
#[tokio::test]
async fn thread_window_shows_latest_replies_and_page_budget_is_explicit() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::connect(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("bounded.db").display()
    ))
    .await
    .unwrap();
    s.migrate().await.unwrap();
    let (_, token) = s.provision("bounded", "owner").await.unwrap();
    let app = router(s);
    let thread = ok(
        app.clone(),
        "POST",
        "/api/v1/threads",
        &token,
        "first",
        json!({"title":"Window","payload":{"text":"oldest"}}),
    )
    .await;
    let id = thread["thread_id"].as_str().unwrap();
    for index in 1..52 {
        ok(
            app.clone(),
            "POST",
            &format!("/api/v1/threads/{id}/messages"),
            &token,
            &format!("reply-{index}"),
            json!({"payload":{"text":format!("reply {index}")}}),
        )
        .await;
    }
    let preview = page(app.clone(), &token, None, 50).await;
    let data = &preview["items"][0]["data"];
    assert_eq!(data["messages"].as_array().unwrap().len(), 50);
    assert_eq!(data["messages"][0]["payload"]["text"], "reply 2");
    assert_eq!(data["messages"][49]["payload"]["text"], "reply 51");
    assert_eq!(data["truncated"], true);
    assert_eq!(data["earlier_messages_omitted"], true);
    assert_eq!(data["lookup"], format!("/api/v1/threads/{id}/messages"));
    assert!(data["next_cursor"].as_str().unwrap().ends_with(":52"));
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/threads/{id}/messages"),
        &token,
        "large",
        json!({"payload":{"text":"한".repeat(60_000)}}),
    )
    .await;
    // Unicode character budget includes JSON escaping, brackets and separators.
    ok(
        app.clone(),
        "POST",
        &format!("/api/v1/threads/{id}/messages"),
        &token,
        "large2",
        json!({"payload":{"text":"x".repeat(100_000)}}),
    )
    .await;
    let bounded = page(app, &token, Some(cursor(&preview)), 50).await;
    assert_eq!(bounded["items"][0]["incomplete"], true);
    assert_eq!(
        bounded["items"][0]["data"]["omission_reason"],
        "page_content_budget"
    );
    assert!(bounded["items"].to_string().chars().count() <= 128_000);
    assert!(bounded["content_chars"].as_u64().unwrap() <= 128_000);
}
