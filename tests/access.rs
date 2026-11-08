use agent_board::{Role, Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn req(a: axum::Router, m: &str, p: &str, t: &str, b: Value) -> (StatusCode, Value) {
    let r = a
        .oneshot(
            Request::builder()
                .method(m)
                .uri(p)
                .header("authorization", format!("Bearer {t}"))
                .header("content-type", "application/json")
                .header("idempotency-key", "access-test")
                .body(Body::from(b.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn contract(url: &str) {
    let s = Store::connect(url).await.unwrap();
    s.migrate().await.unwrap();
    let (p, admin) = s.provision("access", "owner").await.unwrap();
    let reader = s.grant_role(&p, "reader", Role::Reader).await.unwrap();
    let writer = s.grant_role(&p, "writer", Role::Contributor).await.unwrap();
    let (_, other) = s.provision("other", "owner").await.unwrap();
    let a = router(s);
    assert_eq!(
        req(a.clone(), "GET", "/api/v1/events", &reader, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            a.clone(),
            "POST",
            "/api/v1/threads",
            &reader,
            json!({"title":"Forbidden","payload":{}})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req(
            a.clone(),
            "POST",
            "/api/v1/reviews/fake/verdicts",
            &writer,
            json!({"expected_version":1,"verdict":"approved"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for token in [&reader, &writer] {
        assert_eq!(
            req(a.clone(), "GET", "/api/v1/tokens", token, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let (_, list) = req(a.clone(), "GET", "/api/v1/tokens", &admin, Value::Null).await;
    assert!(!list.to_string().contains(&reader));
    let id = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["principal"] == "reader")
        .unwrap()["token_id"]
        .as_str()
        .unwrap();
    let path = format!("/api/v1/tokens/{id}/revoke");
    assert_eq!(
        req(a.clone(), "POST", &path, &other, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        req(a.clone(), "POST", &path, &admin, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        req(a.clone(), "POST", &path, &admin, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        req(a.clone(), "GET", "/api/v1/events", &reader, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        req(
            a,
            "POST",
            "/api/v1/threads",
            &writer,
            json!({"title":"Allowed","payload":{}})
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn sqlite_access() {
    let d = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("a.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_access() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
