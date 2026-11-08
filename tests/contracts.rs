use agent_board::{Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn request(
    app: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let res = app
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
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn contract(url: &str) {
    let s = Store::connect(url).await.unwrap();
    s.migrate().await.unwrap();
    s.migrate().await.unwrap();
    let (project, token) = s.provision("one", "lead").await.unwrap();
    let (_, other) = s.provision("two", "lead").await.unwrap();
    let reply_token = s
        .grant_role(&project, "reviewer", agent_board::Role::Reviewer)
        .await
        .unwrap();
    let app = router(s);
    assert_eq!(
        request(
            app.clone(),
            "GET",
            "/api/v1/events",
            "invalid",
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let body = json!({"title":"한글 논의","payload":{"question":"검토 요청"}});
    let (status, created) = request(
        app.clone(),
        "POST",
        "/api/v1/threads",
        &token,
        "create",
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(
        created,
        request(
            app.clone(),
            "POST",
            "/api/v1/threads",
            &token,
            "create",
            body
        )
        .await
        .1
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/api/v1/threads",
            &token,
            "create",
            json!({"title":"changed","payload":{}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let path = format!(
        "/api/v1/threads/{}/messages",
        created["thread_id"].as_str().unwrap()
    );
    assert_eq!(
        request(app.clone(), "GET", &path, &other, "", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            app.clone(),
            "POST",
            &path,
            &other,
            "bad",
            json!({"payload":{}})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut handles = vec![];
    for _ in 0..8 {
        let app = app.clone();
        let path = path.clone();
        let token = reply_token.clone();
        handles.push(tokio::spawn(async move {
            request(
                app,
                "POST",
                &path,
                &token,
                "reply",
                json!({"payload":{"answer":"ok"}}),
            )
            .await
        }));
    }
    for h in handles {
        let (status, v) = h.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["sequence"], 2);
    }
    let events = request(
        app.clone(),
        "GET",
        "/api/v1/events?limit=1",
        &token,
        "",
        Value::Null,
    )
    .await
    .1;
    assert_eq!(events["items"].as_array().unwrap().len(), 1);
    let cursor = events["next_cursor"].as_str().unwrap().to_owned();
    assert!(cursor.ends_with(":1"));
    assert_eq!(
        request(
            app.clone(),
            "GET",
            &format!("/api/v1/events?after={cursor}"),
            &other,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let events = request(
        app.clone(),
        "GET",
        &format!("/api/v1/events?after={cursor}"),
        &token,
        "",
        Value::Null,
    )
    .await
    .1;
    assert_eq!(events["items"].as_array().unwrap().len(), 1);
    assert!(events["next_cursor"].as_str().unwrap().ends_with(":2"));
    let read = request(app.clone(), "GET", &path, &token, "", Value::Null)
        .await
        .1;
    assert_eq!(read["items"].as_array().unwrap().len(), 2);
    assert_eq!(read["items"][1]["payload"]["answer"], "ok");
    let other_events = request(
        app.clone(),
        "GET",
        "/api/v1/events",
        &other,
        "",
        Value::Null,
    )
    .await
    .1;
    assert!(other_events["items"].as_array().unwrap().is_empty());
    assert_eq!(
        request(
            app,
            "GET",
            "/api/v1/events?limit=201",
            &token,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
#[tokio::test]
async fn sqlite_contract() {
    let dir = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("test.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires dedicated PostgreSQL database via TEST_DATABASE_URL"]
async fn postgres_contract() {
    contract(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required")).await;
}
