use agent_board::{Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn req(a: axum::Router, m: &str, p: &str, t: &str, k: &str, b: Value) -> (StatusCode, Value) {
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
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn ok(a: axum::Router, m: &str, p: &str, t: &str, k: &str, b: Value) -> Value {
    let (s, v) = req(a, m, p, t, k, b).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["data"].clone()
}
async fn contract(url: &str) {
    let s = Store::connect(url).await.unwrap();
    s.migrate().await.unwrap();
    let (project, owner) = s.provision("workflow", "owner").await.unwrap();
    let reviewer = s
        .grant_role(&project, "reviewer", agent_board::Role::Reviewer)
        .await
        .unwrap();
    let (_, outsider) = s.provision("other", "other").await.unwrap();
    let a = router(s);
    let task = ok(
        a.clone(),
        "POST",
        "/api/v1/tasks",
        &owner,
        "new",
        json!({"title":"Task","description":"Criteria"}),
    )
    .await;
    let id = task["id"].as_str().unwrap();
    let path = format!("/api/v1/tasks/{id}");
    assert_eq!(
        req(a.clone(), "GET", &path, &outsider, "", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let input = json!({"expected_version":1,"expected_epoch":0,"run_id":"run","lease_seconds":60});
    let mut joins = vec![];
    for i in 0..4 {
        let a = a.clone();
        let token = owner.clone();
        let p = format!("{path}/claims");
        let b = input.clone();
        joins.push(tokio::spawn(async move {
            req(a, "POST", &p, &token, &format!("claim-{i}"), b).await
        }));
    }
    let mut wins = 0;
    for j in joins {
        if j.await.unwrap().0 == StatusCode::OK {
            wins += 1;
        }
    }
    assert_eq!(wins, 1);
    let patch = json!({"expected_version":2,"run_id":"run","claim_epoch":1,"status":"done"});
    assert_eq!(
        req(
            a.clone(),
            "PATCH",
            &path,
            &owner,
            "done-early",
            patch.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let review=ok(a.clone(),"POST","/api/v1/reviews",&owner,"review",json!({"subject_kind":"task","subject_id":id,"subject_revision":1,"expected_version":2,"reviewer":"reviewer"})).await;
    let verdict = format!(
        "/api/v1/reviews/{}/verdicts",
        review["id"].as_str().unwrap()
    );
    assert_eq!(
        req(
            a.clone(),
            "POST",
            &verdict,
            &owner,
            "wrong-reviewer",
            json!({"expected_version":1,"verdict":"approved"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    ok(
        a.clone(),
        "POST",
        &verdict,
        &reviewer,
        "approve",
        json!({"expected_version":1,"verdict":"approved"}),
    )
    .await;
    let done = ok(a.clone(), "PATCH", &path, &owner, "done", patch).await;
    assert_eq!(done["status"], "done");
    let expiring = ok(
        a.clone(),
        "POST",
        "/api/v1/tasks",
        &owner,
        "expiring",
        json!({"title":"Lease","description":"Expiry"}),
    )
    .await;
    let ep = format!("/api/v1/tasks/{}", expiring["id"].as_str().unwrap());
    ok(
        a.clone(),
        "POST",
        &format!("{ep}/claims"),
        &owner,
        "short",
        json!({"expected_version":1,"expected_epoch":0,"run_id":"old","lease_seconds":1}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        req(
            a.clone(),
            "PATCH",
            &ep,
            &owner,
            "expired",
            json!({"expected_version":2,"run_id":"old","claim_epoch":1,"status":"blocked"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let reclaimed = ok(
        a.clone(),
        "POST",
        &format!("{ep}/claims"),
        &reviewer,
        "reclaim",
        json!({"expected_version":2,"expected_epoch":1,"run_id":"new","lease_seconds":60}),
    )
    .await;
    assert_eq!(reclaimed["lease_epoch"], 2);
    assert_eq!(
        req(
            a.clone(),
            "PATCH",
            &ep,
            &owner,
            "old-epoch",
            json!({"expected_version":3,"run_id":"old","claim_epoch":1,"status":"blocked"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let k = ok(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "note",
        json!({"title":"Note","body":"Original evidence"}),
    )
    .await;
    let kid = k["id"].as_str().unwrap();
    let kp = format!("/api/v1/knowledge/{kid}");
    let source = ok(
        a.clone(),
        "POST",
        "/api/v1/sources",
        &owner,
        "note-source",
        json!({"uri":"urn:evidence:note","revision":"1","label":"Note evidence"}),
    )
    .await;
    ok(
        a.clone(),
        "POST",
        &format!("{kp}/sources"),
        &owner,
        "note-attach",
        json!({"source_id":source["id"],"expected_version":1,"expected_content_revision":1}),
    )
    .await;
    let r=ok(a.clone(),"POST","/api/v1/reviews",&owner,"note-review",json!({"subject_kind":"knowledge","subject_id":kid,"subject_revision":2,"expected_version":2,"reviewer":"reviewer"})).await;
    let v = ok(
        a.clone(),
        "POST",
        &format!("/api/v1/reviews/{}/verdicts", r["id"].as_str().unwrap()),
        &reviewer,
        "note-approve",
        json!({"expected_version":1,"verdict":"approved"}),
    )
    .await;
    assert_eq!(v["subject"]["status"], "verified");
    let revised = ok(
        a.clone(),
        "POST",
        &format!("{kp}/revisions"),
        &owner,
        "revise",
        json!({"expected_version":3,"title":"Note","body":"Changed evidence"}),
    )
    .await;
    assert_eq!(revised["status"], "draft");
    assert_eq!(revised["content_revision"], 3);
    let history = ok(
        a.clone(),
        "GET",
        &format!("{kp}?version=1"),
        &owner,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(history["body"], "Original evidence");
    let pending=ok(a.clone(),"POST","/api/v1/reviews",&owner,"pending",json!({"subject_kind":"knowledge","subject_id":kid,"subject_revision":3,"expected_version":4,"reviewer":"reviewer"})).await;
    ok(
        a.clone(),
        "POST",
        &format!("{kp}/revisions"),
        &owner,
        "edit-again",
        json!({"expected_version":4,"title":"Note","body":"Newest"}),
    )
    .await;
    assert_eq!(
        req(
            a.clone(),
            "POST",
            &format!(
                "/api/v1/reviews/{}/verdicts",
                pending["id"].as_str().unwrap()
            ),
            &reviewer,
            "stale",
            json!({"expected_version":1,"verdict":"approved"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, events) = req(a, "GET", "/api/v1/events", &owner, "", Value::Null).await;
    assert!(!events.to_string().contains("Original evidence"));
}
#[tokio::test]
async fn sqlite_workflows() {
    let d = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        d.path().join("wf.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_workflows() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
