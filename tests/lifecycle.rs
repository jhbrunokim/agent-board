use agent_board::Store;
use sqlx::Row;

#[tokio::test]
async fn startup_requires_current_schema_and_rejects_future_version() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("schema.db").display()
    );
    let store = Store::connect(&url).await.unwrap();
    assert!(store.check_schema().await.is_err());
    store.migrate().await.unwrap();
    store.check_schema().await.unwrap();
    let (id, _) = store.provision("preserved", "owner").await.unwrap();
    store.migrate().await.unwrap();
    let db = sqlx::AnyPool::connect(&url).await.unwrap();
    let row = sqlx::query("SELECT name FROM projects WHERE id = ?")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("name"), "preserved");
    sqlx::query("INSERT INTO schema_version(version) VALUES (999)")
        .execute(&db)
        .await
        .unwrap();
    assert!(store.check_schema().await.is_err());
    assert!(store.migrate().await.is_err());
}

#[test]
fn cli_database_password_is_not_in_help() {
    let bin = env!("CARGO_BIN_EXE_agent-board");
    for command in ["serve", "migrate", "provision", "grant"] {
        let output = std::process::Command::new(bin)
            .args([command, "--help"])
            .env(
                "DATABASE_URL",
                "postgres://example:sentinel-secret@localhost/test",
            )
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("sentinel-secret"));
    }
}

#[tokio::test]
async fn upgrades_v1_without_losing_messages_or_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("v1.db").display());
    let store = Store::connect(&url).await.unwrap();
    let db = sqlx::AnyPool::connect(&url).await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/v1.sql"))
        .execute(&db)
        .await
        .unwrap();
    let (project, _) = store.provision("existing", "owner").await.unwrap();
    sqlx::query("INSERT INTO threads(id,project_id,title) VALUES ('old-thread',?,'Existing')")
        .bind(&project)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages(id,project_id,thread_id,sender,seq,payload) VALUES ('old-message',?,'old-thread','owner',1,'{}')").bind(&project).execute(&db).await.unwrap();
    assert!(store.check_schema().await.is_err());
    store.migrate().await.unwrap();
    store.check_schema().await.unwrap();
    let row = sqlx::query("SELECT payload FROM messages WHERE id='old-message'")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("payload"), "{}");
    let row = sqlx::query("SELECT COUNT(*) AS n FROM tokens WHERE project_id=?")
        .bind(project)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("n"), 1);
}
