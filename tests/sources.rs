use agent_board::{Role, Store, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
async fn req(
    a: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = a
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}
async fn ok(
    a: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    key: &str,
    body: Value,
) -> Value {
    let (status, body) = req(a, method, path, token, key, body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn contract(url: &str) {
    let store = Store::connect(url).await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    store.check_schema().await.unwrap();
    let (project, owner) = store.provision("evidence", "author").await.unwrap();
    let reviewer = store
        .grant_role(&project, "reviewer", Role::Reviewer)
        .await
        .unwrap();
    let reader = store
        .grant_role(&project, "reader", Role::Reader)
        .await
        .unwrap();
    let (_, outsider) = store.provision("other", "other").await.unwrap();
    let a = router(store.clone());
    let input = json!({"uri":"http://127.0.0.1:1/unreachable-secret-reference", "revision":"commit:abc123", "label":"Evidence"});
    let source = ok(
        a.clone(),
        "POST",
        "/api/v1/sources",
        &owner,
        "source",
        input.clone(),
    )
    .await;
    assert_eq!(
        source,
        ok(
            a.clone(),
            "POST",
            "/api/v1/sources",
            &owner,
            "source",
            input.clone()
        )
        .await
    );
    let sid = source["data"]["id"].as_str().unwrap();
    let sp = format!("/api/v1/sources/{sid}");
    assert_eq!(
        ok(a.clone(), "GET", &sp, &reader, "", Value::Null).await["data"],
        source["data"]
    );
    assert_eq!(
        req(a.clone(), "GET", &sp, &outsider, "", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        req(
            a.clone(),
            "POST",
            "/api/v1/sources",
            &reader,
            "reader",
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut changed = input.clone();
    changed["revision"] = json!("commit:changed");
    assert_eq!(
        req(
            a.clone(),
            "POST",
            "/api/v1/sources",
            &owner,
            "source",
            changed.clone()
        )
        .await
        .1["code"],
        "idempotency_conflict"
    );
    assert_eq!(
        req(
            a.clone(),
            "POST",
            "/api/v1/sources",
            &owner,
            "invalid",
            json!({"uri":"relative/path", "revision":"r", "label":"L"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // A source's revision is immutable: a corrected reference gets its own identifier.
    let second = ok(
        a.clone(),
        "POST",
        "/api/v1/sources",
        &owner,
        "source2",
        changed,
    )
    .await;
    assert_ne!(source["data"]["id"], second["data"]["id"]);
    assert_eq!(
        ok(a.clone(), "GET", &sp, &owner, "", Value::Null).await["data"]["revision"],
        "commit:abc123"
    );
    let knowledge = ok(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "knowledge",
        json!({"title":"Finding", "body":"A supported finding"}),
    )
    .await;
    let id = knowledge["data"]["id"].as_str().unwrap();
    let kp = format!("/api/v1/knowledge/{id}");
    let ap = format!("{kp}/sources");
    let pending = ok(a.clone(), "POST", "/api/v1/reviews", &owner, "unsupported-review", json!({"subject_kind":"knowledge", "subject_id":id, "subject_revision":1, "expected_version":1, "reviewer":"reviewer"})).await;
    let vp = format!(
        "/api/v1/reviews/{}/verdicts",
        pending["data"]["id"].as_str().unwrap()
    );
    let (status, failure) = req(
        a.clone(),
        "POST",
        &vp,
        &reviewer,
        "unsupported-approve",
        json!({"expected_version":1, "verdict":"approved"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(failure["code"], "sources_required");
    let attachment = json!({"source_id":sid, "expected_version":1, "expected_content_revision":1});
    assert_eq!(
        req(
            a.clone(),
            "POST",
            &ap,
            &outsider,
            "foreign",
            attachment.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let foreign_source = ok(
        a.clone(),
        "POST",
        "/api/v1/sources",
        &outsider,
        "foreign-source",
        input.clone(),
    )
    .await;
    let mut bad = attachment.clone();
    bad["source_id"] = foreign_source["data"]["id"].clone();
    assert_eq!(
        req(a.clone(), "POST", &ap, &owner, "foreign-link", bad)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let mut stale = attachment.clone();
    stale["expected_content_revision"] = json!(2);
    assert_eq!(
        req(a.clone(), "POST", &ap, &owner, "stale-revision", stale)
            .await
            .1["code"],
        "content_revision_conflict"
    );
    let attached = ok(a.clone(), "POST", &ap, &owner, "attach", attachment.clone()).await;
    assert_eq!(
        attached,
        ok(a.clone(), "POST", &ap, &owner, "attach", attachment).await
    );
    assert_eq!(attached["data"]["version"], 2);
    assert_eq!(attached["data"]["content_revision"], 2);
    assert_eq!(attached["data"]["status"], "draft");
    assert_eq!(attached["data"]["sources"].as_array().unwrap().len(), 1);
    // Review pinned to revision 1 cannot approve revision 2 after an evidence change.
    assert_eq!(
        req(
            a.clone(),
            "POST",
            &vp,
            &reviewer,
            "stale-approve",
            json!({"expected_version":1, "verdict":"approved"})
        )
        .await
        .1["code"],
        "stale_review"
    );
    assert!(
        ok(
            a.clone(),
            "GET",
            &format!("{kp}?version=1"),
            &reader,
            "",
            Value::Null
        )
        .await["data"]["sources"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(a.clone(), "GET", &kp, &reader, "", Value::Null).await["data"]["sources"][0]["id"],
        sid
    );
    // Duplicate attachment is a conflict, without creating an otherwise identical revision.
    assert_eq!(
        req(
            a.clone(),
            "POST",
            &ap,
            &owner,
            "duplicate",
            json!({"source_id":sid,"expected_version":2,"expected_content_revision":2})
        )
        .await
        .1["code"],
        "source_already_attached"
    );
    let approved = ok(a.clone(), "POST", "/api/v1/reviews", &owner, "supported-review", json!({"subject_kind":"knowledge","subject_id":id,"subject_revision":2,"expected_version":2,"reviewer":"reviewer"})).await;
    let avp = format!(
        "/api/v1/reviews/{}/verdicts",
        approved["data"]["id"].as_str().unwrap()
    );
    let verified = ok(
        a.clone(),
        "POST",
        &avp,
        &reviewer,
        "supported-approve",
        json!({"expected_version":1,"verdict":"approved"}),
    )
    .await;
    assert_eq!(verified["data"]["subject"]["status"], "verified");
    let sid2 = second["data"]["id"].as_str().unwrap();
    // Competing attachments use the same optimistic version. Exactly one commits.
    let third = ok(
        a.clone(),
        "POST",
        "/api/v1/sources",
        &owner,
        "source3",
        json!({"uri":"urn:evidence:third","revision":"1","label":"Third"}),
    )
    .await;
    let sid3 = third["data"]["id"].as_str().unwrap();
    let first = req(
        a.clone(),
        "POST",
        &ap,
        &owner,
        "race1",
        json!({"source_id":sid2,"expected_version":3,"expected_content_revision":2}),
    );
    let other = req(
        a.clone(),
        "POST",
        &ap,
        &owner,
        "race2",
        json!({"source_id":sid3,"expected_version":3,"expected_content_revision":2}),
    );
    let (one, two) = tokio::join!(first, other);
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|&&s| s == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [one.0, two.0]
            .iter()
            .filter(|&&s| s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let latest = ok(a.clone(), "GET", &kp, &reader, "", Value::Null).await;
    assert_eq!(latest["data"]["status"], "draft");
    assert_eq!(latest["data"]["version"], 4);
    assert_eq!(latest["data"]["sources"].as_array().unwrap().len(), 2);
    assert_eq!(
        ok(
            a.clone(),
            "GET",
            &format!("{kp}?version=3"),
            &reader,
            "",
            Value::Null
        )
        .await["data"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Text revisions also carry evidence forward, while resetting verification.
    let revised = ok(
        a.clone(),
        "POST",
        &format!("{kp}/revisions"),
        &owner,
        "text-revision",
        json!({"expected_version":4,"title":"Finding","body":"Revised finding"}),
    )
    .await;
    assert_eq!(revised["data"]["content_revision"], 4);
    assert_eq!(
        ok(a.clone(), "GET", &kp, &reader, "", Value::Null).await["data"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let events = ok(a.clone(), "GET", "/api/v1/events", &owner, "", Value::Null).await;
    let items = events["items"].as_array().unwrap();
    let sequences: Vec<i64> = items
        .iter()
        .map(|e| e["sequence"].as_i64().unwrap())
        .collect();
    assert_eq!(sequences, (1..=sequences.len() as i64).collect::<Vec<_>>());
    assert_eq!(
        items
            .iter()
            .filter(|e| e["type"] == "source:create")
            .count(),
        3
    );
    assert_eq!(
        items
            .iter()
            .filter(|e| e["type"]
                .as_str()
                .is_some_and(|t| t.starts_with("source:attach:")))
            .count(),
        2
    );
    assert!(!events.to_string().contains("unreachable-secret-reference"));
    assert!(!events.to_string().contains("A supported finding"));
    // A creates; B writes body; C adds evidence. Neither B nor C may review.
    let body_author = store
        .grant_role(&project, "body-author", Role::Reviewer)
        .await
        .unwrap();
    let attacher = store
        .grant_role(&project, "evidence-author", Role::Reviewer)
        .await
        .unwrap();
    let separate = ok(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "independence-new",
        json!({"title":"Separate", "body":"A body"}),
    )
    .await;
    let separate_id = separate["data"]["id"].as_str().unwrap();
    let separate_path = format!("/api/v1/knowledge/{separate_id}");
    ok(
        a.clone(),
        "POST",
        &format!("{separate_path}/revisions"),
        &body_author,
        "independence-body",
        json!({"expected_version":1,"title":"Separate","body":"B body"}),
    )
    .await;
    let linked = ok(
        a.clone(),
        "POST",
        &format!("{separate_path}/sources"),
        &attacher,
        "independence-evidence",
        json!({"expected_version":2,"expected_content_revision":2,"source_id":sid}),
    )
    .await;
    assert_eq!(linked["data"]["content_author"], "body-author");
    for reviewer_name in ["body-author", "evidence-author"] {
        let failed = req(a.clone(), "POST", "/api/v1/reviews", &owner, &format!("independence-{reviewer_name}"), json!({"subject_kind":"knowledge","subject_id":separate_id,"subject_revision":3,"expected_version":3,"reviewer":reviewer_name})).await;
        assert_eq!(failed.0, StatusCode::CONFLICT);
        assert_eq!(failed.1["code"], "independent_reviewer_required");
    }
    // Copied evidence retains its attachment participant after a later body edit.
    ok(
        a.clone(),
        "POST",
        &format!("{separate_path}/revisions"),
        &body_author,
        "independence-body2",
        json!({"expected_version":3,"title":"Separate","body":"B revised body"}),
    )
    .await;
    assert_eq!(req(a.clone(), "POST", "/api/v1/reviews", &owner, "independence-copied", json!({"subject_kind":"knowledge","subject_id":separate_id,"subject_revision":4,"expected_version":4,"reviewer":"evidence-author"})).await.1["code"], "independent_reviewer_required");
    let independent = ok(a.clone(), "POST", "/api/v1/reviews", &owner, "independence-valid", json!({"subject_kind":"knowledge","subject_id":separate_id,"subject_revision":4,"expected_version":4,"reviewer":"reviewer"})).await;
    ok(
        a.clone(),
        "POST",
        &format!(
            "/api/v1/reviews/{}/verdicts",
            independent["data"]["id"].as_str().unwrap()
        ),
        &reviewer,
        "independence-approved",
        json!({"expected_version":1,"verdict":"approved"}),
    )
    .await;
    let capped = ok(
        a.clone(),
        "POST",
        "/api/v1/knowledge",
        &owner,
        "cap-new",
        json!({"title":"Bounded evidence","body":"Evidence cap"}),
    )
    .await;
    let capped_path = format!(
        "/api/v1/knowledge/{}/sources",
        capped["data"]["id"].as_str().unwrap()
    );
    for index in 0..17 {
        let source = ok(
            a.clone(),
            "POST",
            "/api/v1/sources",
            &owner,
            &format!("cap-source-{index}"),
            json!({"uri":format!("urn:cap:{index}"),"revision":"1","label":"Bounded"}),
        )
        .await;
        let result = req(a.clone(), "POST", &capped_path, &owner, &format!("cap-attach-{index}"), json!({"expected_version":index+1,"expected_content_revision":index+1,"source_id":source["data"]["id"]})).await;
        if index < 16 {
            assert_eq!(result.0, StatusCode::OK);
        } else {
            assert_eq!(result.0, StatusCode::CONFLICT);
            assert_eq!(result.1["code"], "source_limit");
        }
    }
    let capped_read = ok(
        a,
        "GET",
        &format!(
            "/api/v1/knowledge/{}",
            capped["data"]["id"].as_str().unwrap()
        ),
        &reader,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(capped_read["data"]["version"], 17);
    assert_eq!(capped_read["data"]["sources"].as_array().unwrap().len(), 16);
}
#[tokio::test]
async fn sqlite_sources() {
    let dir = tempfile::tempdir().unwrap();
    contract(&format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("sources.db").display()
    ))
    .await;
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn postgres_sources() {
    contract(&std::env::var("TEST_DATABASE_URL").unwrap()).await;
}
