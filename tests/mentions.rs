use agent_board::{Role, Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    app: &axum::Router,
    token: &str,
    method: &str,
    path: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
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

async fn contract(url: &str) {
    let store = Store::connect(url).await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let (project, admin) = store.provision("Mentions", "admin").await.unwrap();
    let author = store.grant(&project, "author").await.unwrap();
    let leader = store
        .grant_role(&project, "lead", Role::Reader)
        .await
        .unwrap();
    let new_leader = store
        .grant_role(&project, "next", Role::Reader)
        .await
        .unwrap();
    let (other_project, other_admin) = store.provision("Other", "lead").await.unwrap();
    let app = router(store.clone());

    let roster = json!({
        "expected_version": 0, "leader": "lead",
        "members": [{"handle":"backend","principal":"author"},{"handle":"qa","principal":"lead"}]
    });
    let team = "/api/v1/mention-teams/platform";
    assert_eq!(
        request(&app, &author, "POST", team, "roster", roster.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let configured = request(&app, &admin, "POST", team, "roster", roster.clone()).await;
    assert_eq!(configured.0, StatusCode::OK, "{}", configured.1);
    assert_eq!(
        request(&app, &admin, "POST", team, "roster", roster.clone()).await,
        configured
    );
    assert_eq!(
        request(&app, &admin, "POST", team, "stale-roster", roster)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, directory) = request(&app, &admin, "GET", team, "", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(directory["version"], 1);
    assert_eq!(directory["members"].as_array().unwrap().len(), 2);

    let unknown = json!({"title":"Invalid recipient","mentions":["unknown"],"payload":{}});
    let rejected = request(&app, &author, "POST", "/api/v1/threads", "unknown", unknown).await;
    assert_eq!(rejected.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rejected.1["code"], "unknown_mention");

    let message = json!({"title":"@backend 검토","payload":{"question":"@qa @backend 부탁해요"}});
    let (status, posted) = request(
        &app,
        &author,
        "POST",
        "/api/v1/threads",
        "post",
        message.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    assert_eq!(posted["sequence"], 1);
    assert_eq!(posted["notifications_queued"], 1);
    assert_eq!(
        request(&app, &author, "POST", "/api/v1/threads", "post", message)
            .await
            .1,
        posted
    );
    let (_, inbox) = request(&app, &leader, "GET", "/api/v1/inbox", "", Value::Null).await;
    assert_eq!(inbox["items"].as_array().unwrap().len(), 1);
    let item = &inbox["items"][0];
    assert_eq!(item["mentions"], json!(["backend", "qa"]));
    assert_eq!(item["sender"], "author");
    assert_eq!(item["message_id"], posted["message_id"]);
    assert!(item.get("payload").is_none());
    let (_, original) = request(
        &app,
        &leader,
        "GET",
        item["lookup"].as_str().unwrap(),
        "",
        Value::Null,
    )
    .await;
    assert_eq!(original["items"][0]["id"], posted["message_id"]);

    for token in [&author, &new_leader, &other_admin] {
        let (_, empty) = request(&app, token, "GET", "/api/v1/inbox", "", Value::Null).await;
        assert!(empty["items"].as_array().unwrap().is_empty());
    }
    let cursor_path = format!(
        "/api/v1/inbox?after={}",
        inbox["next_cursor"].as_str().unwrap()
    );
    assert_eq!(
        request(&app, &new_leader, "GET", &cursor_path, "", Value::Null)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let ack = format!("/api/v1/inbox/{}/ack", item["id"].as_str().unwrap());
    for token in [&admin, &author, &other_admin] {
        assert_eq!(
            request(&app, token, "POST", &ack, "", json!({})).await.0,
            StatusCode::NOT_FOUND
        );
    }
    for _ in 0..2 {
        assert_eq!(
            request(&app, &leader, "POST", &ack, "", json!({})).await.0,
            StatusCode::OK
        );
    }
    let (_, empty) = request(&app, &leader, "GET", "/api/v1/inbox", "", Value::Null).await;
    assert!(empty["items"].as_array().unwrap().is_empty());
    let (_, history) = request(
        &app,
        &leader,
        "GET",
        "/api/v1/inbox?unread=false",
        "",
        Value::Null,
    )
    .await;
    assert_eq!(history["items"][0]["acknowledged"], true);

    let moved = json!({"expected_version":1,"leader":"next","members":[{"handle":"backend","principal":"author"}]});
    assert_eq!(
        request(&app, &admin, "POST", team, "move", moved).await.0,
        StatusCode::OK
    );
    let thread_id = posted["thread_id"].as_str().unwrap();
    let reply_path = format!("/api/v1/threads/{thread_id}/messages");

    let quiet = json!({"payload":{"answer":"`@backend` and me@backend and @unknown"}});
    let (status, quiet) = request(&app, &author, "POST", &reply_path, "quiet", quiet).await;
    assert_eq!(status, StatusCode::OK);
    assert!(quiet.get("notifications_queued").is_none());
    let explicit = json!({"payload":{"answer":"다시 검토 부탁해요"},"mentions":["backend"]});
    let (status, reply) = request(&app, &author, "POST", &reply_path, "reply", explicit).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["sequence"], 3);
    let (_, routed) = request(&app, &new_leader, "GET", "/api/v1/inbox", "", Value::Null).await;
    assert_eq!(routed["items"].as_array().unwrap().len(), 1);
    assert_eq!(routed["items"][0]["message_id"], reply["message_id"]);
    let (_, old) = request(
        &app,
        &leader,
        "GET",
        "/api/v1/inbox?unread=false",
        "",
        Value::Null,
    )
    .await;
    assert_eq!(old["items"].as_array().unwrap().len(), 1);

    let foreign = json!({"expected_version":0,"leader":"next","members":[{"handle":"backend","principal":"lead"}]});
    assert_eq!(
        request(&app, &other_admin, "POST", team, "foreign", foreign)
            .await
            .1["code"],
        "unknown_principal"
    );
    let foreign = json!({"expected_version":0,"leader":"lead","members":[{"handle":"backend","principal":"lead"}]});
    assert_eq!(
        request(&app, &other_admin, "POST", team, "local", foreign)
            .await
            .0,
        StatusCode::OK
    );
    assert_ne!(project, other_project);
}

#[tokio::test]
async fn sqlite_mentions() {
    let dir = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("mentions.db").display()
    ))
    .await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_mentions() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
