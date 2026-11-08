//! Project-scoped token roles and revocation, without plaintext credential persistence.
use super::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Reader,
    Contributor,
    Reviewer,
    Admin,
}
impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Contributor => "contributor",
            Self::Reviewer => "reviewer",
            Self::Admin => "admin",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        match s {
            "reader" => Ok(Self::Reader),
            "contributor" => Ok(Self::Contributor),
            "reviewer" => Ok(Self::Reviewer),
            "admin" => Ok(Self::Admin),
            _ => Err(Error(StatusCode::FORBIDDEN, "unknown_role")),
        }
    }
}
pub(super) fn allowed(who: &Identity, method: &str, path: &str) -> bool {
    if path.starts_with("/api/v1/tokens") {
        return matches!(who.role, Role::Admin);
    }
    match who.role {
        Role::Admin => true,
        Role::Reader => method == "GET",
        Role::Reviewer => true,
        Role::Contributor => !path.starts_with("/api/v1/tokens") && !path.ends_with("/verdicts"),
    }
}
impl Store {
    pub async fn grant_role(
        &self,
        project: &str,
        principal: &str,
        role: Role,
    ) -> anyhow::Result<String> {
        anyhow::ensure!(
            !principal.trim().is_empty() && principal.len() <= 256,
            "invalid principal"
        );
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sqlx::query(
            &self.sql("INSERT INTO tokens(hash,project_id,principal,role) VALUES (?,?,?,?)"),
        )
        .bind(hash(&token))
        .bind(project)
        .bind(principal)
        .bind(role.name())
        .execute(&self.pool)
        .await?;
        Ok(token)
    }
    pub async fn list_tokens(&self, project: &str) -> anyhow::Result<Value> {
        let rows = sqlx::query(&self.sql(
            "SELECT hash,principal,role,revoked FROM tokens WHERE project_id=? ORDER BY hash",
        ))
        .bind(project)
        .fetch_all(&self.pool)
        .await?;
        let items=rows.into_iter().map(|r|Ok(json!({"token_id":r.try_get::<String,_>("hash")?,"principal":r.try_get::<String,_>("principal")?,"role":r.try_get::<String,_>("role")?,"revoked":r.try_get::<i64,_>("revoked")?!=0}))).collect::<std::result::Result<Vec<Value>,sqlx::Error>>()?;
        Ok(json!({"items":items}))
    }
    pub async fn revoke_token(&self, project: &str, id: &str) -> anyhow::Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&self.sql("UPDATE projects SET head=head WHERE id=?"))
            .bind(project)
            .execute(&mut *tx)
            .await?;
        let n = sqlx::query(&self.sql("UPDATE tokens SET revoked=1 WHERE project_id=? AND hash=?"))
            .bind(project)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(n > 0)
    }
    pub(super) async fn active(
        &self,
        who: &Identity,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    ) -> Result<()> {
        let r = sqlx::query(
            &self.sql("SELECT hash FROM tokens WHERE hash=? AND project_id=? AND revoked=0"),
        )
        .bind(&who.token_id)
        .bind(&who.project)
        .fetch_optional(&mut **tx)
        .await?;
        if r.is_none() {
            return Err(Error(StatusCode::UNAUTHORIZED, "unauthorized"));
        }
        Ok(())
    }
}
pub(super) fn parse_role(s: &str) -> Result<Role> {
    Role::parse(s)
}
pub(super) fn routes() -> Router<Store> {
    Router::new()
        .route("/api/v1/tokens", get(list))
        .route("/api/v1/tokens/{id}/revoke", post(revoke))
}
async fn list(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
) -> Result<Json<Value>> {
    Ok(Json(s.list_tokens(&who.project).await.map_err(|_| {
        Error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
    })?))
}
async fn revoke(
    State(s): State<Store>,
    axum::Extension(who): axum::Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let found = s
        .revoke_token(&who.project, &id)
        .await
        .map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"))?;
    if !found {
        return Err(missing());
    }
    Ok(Json(json!({"revoked":true})))
}
