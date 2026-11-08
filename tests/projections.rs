use agent_board::{Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn req(a: axum::Router, m: &str, p: &str, t: &str, k: &str, b: Value) -> Value {
    let r = a
        .oneshot(
            Request::builder()
                .method(m)
                .uri(p)
                .header("authorization", format!("Bearer {t}"))
                .header("idempotency-key", k)
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
async fn contract(url: &str) {
    let s = Store::connect(url).await.unwrap();
    s.migrate().await.unwrap();
    let (_, token) = s.provision("p", "owner").await.unwrap();
    let (_, other) = s.provision("other", "other").await.unwrap();
    let a = router(s);
    req(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &token,
        "note",
        json!({"title":"한글 메모","body":"long evidence ".repeat(100)}),
    )
    .await;
    let found = req(
        a.clone(),
        "GET",
        "/api/v1/search?q=%ED%95%9C%EA%B8%80",
        &token,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    let hidden = req(
        a.clone(),
        "GET",
        "/api/v1/search?q=%ED%95%9C%EA%B8%80",
        &other,
        "",
        Value::Null,
    )
    .await;
    assert!(hidden["items"].as_array().unwrap().is_empty());
    let pack = req(
        a.clone(),
        "GET",
        "/api/v1/context?max_chars=512",
        &token,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(pack["incomplete"], true);
    assert!(pack["items"].as_array().unwrap().is_empty());
    assert_eq!(pack["next_cursor"], Value::Null);
    let all = req(a, "GET", "/api/v1/export", &token, "", Value::Null).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn sqlite_projection() {
    let d = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("p.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_projection() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
#[tokio::test]
async fn vault_preserves_manual_edits() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::connect(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("v.db").display()
    ))
    .await
    .unwrap();
    s.migrate().await.unwrap();
    let (project, token) = s.provision("vault", "owner").await.unwrap();
    let a = router(s);
    req(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &token,
        "note",
        json!({"title":"Note","body":"Evidence"}),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, a).await.unwrap();
    });
    let vault = d.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("Personal.md"), "mine").unwrap();
    assert_eq!(
        agent_board::vault::pull(&url, &token, &vault)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        agent_board::vault::pull(&url, &token, &vault)
            .await
            .unwrap(),
        0
    );
    let dir = vault.join("AgentBoard").join(project);
    let note = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "md"))
        .unwrap();
    std::fs::write(&note, "my edits").unwrap();
    assert!(
        agent_board::vault::pull(&url, &token, &vault)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(note).unwrap(), "my edits");
    assert_eq!(
        std::fs::read_to_string(vault.join("Personal.md")).unwrap(),
        "mine"
    );
    server.abort();
}
