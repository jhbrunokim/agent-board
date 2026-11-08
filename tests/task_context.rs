use agent_board::{Role, Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn req(
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
    let (status, body) = req(app, method, path, token, key, body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
fn assert_budget(context: &Value) {
    let actual = context["items"].to_string().chars().count();
    assert_eq!(context["content_chars"], actual);
    assert!(actual <= context["max_chars"].as_u64().unwrap() as usize);
}
async fn contract(url: &str) {
    let store = Store::connect(url).await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    store.check_schema().await.unwrap();
    let (project, owner) = store.provision("task context", "owner").await.unwrap();
    let reader = store
        .grant_role(&project, "reader", Role::Reader)
        .await
        .unwrap();
    let contributor = store
        .grant_role(&project, "other-writer", Role::Contributor)
        .await
        .unwrap();
    let reviewer = store
        .grant_role(&project, "reviewer", Role::Reviewer)
        .await
        .unwrap();
    let (_, outsider) = store.provision("outside", "outside").await.unwrap();
    let app = router(store.clone());
    let task = ok(
        app.clone(),
        "POST",
        "/api/v1/tasks",
        &owner,
        "task",
        json!({"title":"Focused task","description":"Acceptance criteria"}),
    )
    .await;
    let tid = task["data"]["id"].as_str().unwrap();
    let tp = format!("/api/v1/tasks/{tid}");
    let lp = format!("{tp}/knowledge");
    let cp = format!("{tp}/context");
    let knowledge = ok(
        app.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "knowledge",
        json!({"title":"Pinned finding","body":"Original body with 한글 and \"escaping\""}),
    )
    .await;
    let kid = knowledge["data"]["id"].as_str().unwrap();
    let kp = format!("/api/v1/knowledge/{kid}");
    ok(
        app.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "unrelated",
        json!({"title":"UNRELATED_SECRET","body":"UNRELATED_SECRET"}),
    )
    .await;
    let source = ok(
        app.clone(),
        "POST",
        "/api/v1/sources",
        &owner,
        "source",
        json!({"uri":"urn:reference:pinned","revision":"commit-a","label":"Pinned evidence"}),
    )
    .await;
    ok(app.clone(),"POST",&format!("{kp}/sources"),&owner,"attach",json!({"expected_version":1,"expected_content_revision":1,"source_id":source["data"]["id"]})).await;
    let mut input = json!({"expected_task_version":1,"run_id":"run","claim_epoch":1,"knowledge_id":kid,"knowledge_content_revision":2});
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "no-claim", input.clone())
            .await
            .1["code"],
        "claim_required"
    );
    let claimed = ok(
        app.clone(),
        "POST",
        &format!("{tp}/claims"),
        &owner,
        "claim",
        json!({"expected_version":1,"expected_epoch":0,"run_id":"run","lease_seconds":3600}),
    )
    .await;
    assert_eq!(claimed["data"]["version"], 2);
    input["expected_task_version"] = json!(2);
    for (token, expected) in [
        (&reader, StatusCode::FORBIDDEN),
        (&contributor, StatusCode::FORBIDDEN),
        (&outsider, StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            req(app.clone(), "POST", &lp, token, "bad-scope", input.clone())
                .await
                .0,
            expected
        );
    }
    let mut bad = input.clone();
    bad["claim_epoch"] = json!(2);
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "stale-claim", bad)
            .await
            .1["code"],
        "stale_claim"
    );
    let mut bad = input.clone();
    bad["knowledge_content_revision"] = json!(1);
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "stale-knowledge", bad)
            .await
            .1["code"],
        "content_revision_conflict"
    );
    let mut bad = input.clone();
    bad["knowledge_id"] = json!(tid);
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "wrong-kind", bad)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let foreign = ok(
        app.clone(),
        "POST",
        "/api/v1/knowledge",
        &outsider,
        "foreign",
        json!({"title":"Foreign","body":"Foreign"}),
    )
    .await;
    let mut bad = input.clone();
    bad["knowledge_id"] = foreign["data"]["id"].clone();
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "foreign", bad)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let old_review=ok(app.clone(),"POST","/api/v1/reviews",&owner,"review-r1",json!({"subject_kind":"task","subject_id":tid,"subject_revision":1,"expected_version":2,"reviewer":"reviewer"})).await;
    let (linked, concurrent_replay) = tokio::join!(
        ok(app.clone(), "POST", &lp, &owner, "link", input.clone()),
        ok(app.clone(), "POST", &lp, &owner, "link", input.clone())
    );
    assert_eq!(linked, concurrent_replay);
    assert_eq!(linked["data"]["task"]["version"], 3);
    assert_eq!(linked["data"]["task"]["content_revision"], 1);
    assert_eq!(
        linked,
        ok(app.clone(), "POST", &lp, &owner, "link", input.clone()).await
    );
    let mut duplicate = input.clone();
    duplicate["expected_task_version"] = json!(3);
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "duplicate", duplicate)
            .await
            .1["code"],
        "knowledge_already_linked"
    );
    input["knowledge_content_revision"] = json!(1);
    assert_eq!(
        req(app.clone(), "POST", &lp, &owner, "link", input).await.1["code"],
        "idempotency_conflict"
    );
    ok(
        app.clone(),
        "POST",
        &format!("{kp}/revisions"),
        &owner,
        "revise",
        json!({"expected_version":2,"title":"New finding","body":"UPDATED_BODY"}),
    )
    .await;
    let before = ok(
        app.clone(),
        "GET",
        "/api/v1/events",
        &owner,
        "",
        Value::Null,
    )
    .await;
    let context = ok(app.clone(), "GET", &cp, &reader, "", Value::Null).await;
    assert_budget(&context);
    assert_eq!(context["incomplete"], false);
    assert_eq!(context["items"][0]["kind"], "task");
    assert_eq!(context["items"][1]["id"], old_review["data"]["id"]);
    let pinned = context["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "knowledge")
        .unwrap();
    assert_eq!(pinned["data"]["body"], knowledge["data"]["body"]);
    assert_eq!(pinned["pinned_content_revision"], 2);
    assert_eq!(pinned["current_content_revision"], 3);
    assert_eq!(pinned["stale"], true);
    assert_eq!(pinned["data"]["sources"][0]["id"], source["data"]["id"]);
    assert_eq!(pinned["data"]["sources"][0]["revision"], "commit-a");
    assert!(!context.to_string().contains("UPDATED_BODY"));
    assert!(!context.to_string().contains("UNRELATED_SECRET"));
    assert_eq!(
        context,
        ok(
            app.clone(),
            "GET",
            &format!("/api/v1/context?task_id={tid}"),
            &reader,
            "",
            Value::Null
        )
        .await
    );
    assert_eq!(
        req(app.clone(), "GET", &cp, &outsider, "", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        before,
        ok(
            app.clone(),
            "GET",
            "/api/v1/events",
            &owner,
            "",
            Value::Null
        )
        .await
    );
    // Only reviews of current acceptance/body revision belong in the bundle.
    ok(app.clone(),"PATCH",&tp,&owner,"update",json!({"expected_version":3,"run_id":"run","claim_epoch":1,"description":"New acceptance criteria"})).await;
    let new_review=ok(app.clone(),"POST","/api/v1/reviews",&owner,"review-r2",json!({"subject_kind":"task","subject_id":tid,"subject_revision":2,"expected_version":4,"reviewer":"reviewer"})).await;
    let context = ok(app.clone(), "GET", &cp, &reader, "", Value::Null).await;
    assert_eq!(context["items"][1]["id"], new_review["data"]["id"]);
    assert!(
        !context
            .to_string()
            .contains(old_review["data"]["id"].as_str().unwrap())
    );
    for n in 0..16 {
        ok(app.clone(),"POST","/api/v1/reviews",&owner,&format!("extra-review-{n}"),json!({"subject_kind":"task","subject_id":tid,"subject_revision":2,"expected_version":4,"reviewer":"reviewer"})).await;
    }
    let context = ok(
        app.clone(),
        "GET",
        &format!("{cp}?max_chars=32000"),
        &reader,
        "",
        Value::Null,
    )
    .await;
    assert_budget(&context);
    assert_eq!(
        context["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["kind"] == "review")
            .count(),
        16
    );
    assert_eq!(context["reviews_have_more"], true);
    assert_eq!(context["incomplete"], true);
    assert_eq!(context["omitted_at_least"], 1);
    // Oversized mandatory task cannot be silently dropped and claimed complete.
    let oversized = ok(
        app.clone(),
        "POST",
        "/api/v1/tasks",
        &owner,
        "huge-task",
        json!({"title":"Huge","description":"한글\\\"".repeat(600)}),
    )
    .await;
    let oid = oversized["data"]["id"].as_str().unwrap();
    let context = ok(
        app.clone(),
        "GET",
        &format!("/api/v1/tasks/{oid}/context?max_chars=512"),
        &reader,
        "",
        Value::Null,
    )
    .await;
    assert_budget(&context);
    assert_eq!(context["items"], json!([]));
    assert_eq!(context["incomplete"], true);
    assert_eq!(context["omitted"][0]["id"], oid);
    assert_eq!(context["next_cursor"], Value::Null);
    assert_eq!(context["lookup"], format!("/api/v1/tasks/{oid}"));
    assert_eq!(
        req(
            app.clone(),
            "GET",
            &format!("{cp}?max_chars=511"),
            &reader,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Trust can change without changing content; show both pinned and current status.
    let trust_task = ok(
        app.clone(),
        "POST",
        "/api/v1/tasks",
        &owner,
        "trust-task",
        json!({"title":"Trust task","description":"Inspect review metadata"}),
    )
    .await;
    let trust_tid = trust_task["data"]["id"].as_str().unwrap();
    let trust_tp = format!("/api/v1/tasks/{trust_tid}");
    ok(
        app.clone(),
        "POST",
        &format!("{trust_tp}/claims"),
        &owner,
        "trust-claim",
        json!({"expected_version":1,"expected_epoch":0,"run_id":"trust","lease_seconds":3600}),
    )
    .await;
    let kr=ok(app.clone(),"POST","/api/v1/reviews",&owner,"trust-approve-review",json!({"subject_kind":"knowledge","subject_id":kid,"subject_revision":3,"expected_version":3,"reviewer":"reviewer"})).await;
    ok(
        app.clone(),
        "POST",
        &format!(
            "/api/v1/reviews/{}/verdicts",
            kr["data"]["id"].as_str().unwrap()
        ),
        &reviewer,
        "trust-approve",
        json!({"expected_version":1,"verdict":"approved"}),
    )
    .await;
    ok(app.clone(),"POST",&format!("{trust_tp}/knowledge"),&owner,"trust-link",json!({"expected_task_version":2,"run_id":"trust","claim_epoch":1,"knowledge_id":kid,"knowledge_content_revision":3})).await;
    let kr=ok(app.clone(),"POST","/api/v1/reviews",&owner,"trust-reject-review",json!({"subject_kind":"knowledge","subject_id":kid,"subject_revision":3,"expected_version":4,"reviewer":"reviewer"})).await;
    ok(
        app.clone(),
        "POST",
        &format!(
            "/api/v1/reviews/{}/verdicts",
            kr["data"]["id"].as_str().unwrap()
        ),
        &reviewer,
        "trust-reject",
        json!({"expected_version":1,"verdict":"rejected"}),
    )
    .await;
    let trust = ok(
        app.clone(),
        "GET",
        &format!("{trust_tp}/context"),
        &reader,
        "",
        Value::Null,
    )
    .await;
    let pinned = &trust["items"][1];
    assert_eq!(pinned["status_at_link"], "verified");
    assert_eq!(pinned["current_status"], "draft");
    assert_eq!(pinned["status_changed"], true);
    assert_eq!(pinned["content_changed"], false);
    assert_eq!(pinned["stale"], false);
    assert_eq!(pinned["pinned_version"], 4);
    assert_eq!(pinned["current_version"], 5);
    // Relation cap is enforced in the same lock/version transaction.
    for n in 0..16 {
        let extra = ok(
            app.clone(),
            "POST",
            "/api/v1/knowledge",
            &owner,
            &format!("cap-knowledge-{n}"),
            json!({"title":"Extra","body":"Extra"}),
        )
        .await;
        let result=req(app.clone(),"POST",&lp,&owner,&format!("cap-link-{n}"),json!({"expected_task_version":4+n,"run_id":"run","claim_epoch":1,"knowledge_id":extra["data"]["id"],"knowledge_content_revision":1})).await;
        if n < 15 {
            assert_eq!(result.0, StatusCode::OK, "{}", result.1);
        } else {
            assert_eq!(result.1["code"], "knowledge_limit");
        }
    }
}
#[tokio::test]
async fn sqlite_task_context() {
    let dir = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("context.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_task_context() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
