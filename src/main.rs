use agent_board::{Store, router};
use clap::{Parser, Subcommand};
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Migrate {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
    },
    Provision {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        principal: String,
    },
    Grant {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        principal: String,
    },
    Serve {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, default_value = "127.0.0.1:7431")]
        bind: String,
    },
    Export {
        #[arg(long, default_value = "http://127.0.0.1:7431")]
        url: String,
        #[arg(long, env = "BOARD_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long)]
        vault: std::path::PathBuf,
    },
    Request {
        #[arg(long, default_value = "http://127.0.0.1:7431")]
        url: String,
        #[arg(long, env = "BOARD_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long, value_parser = ["GET", "POST", "PATCH"])]
        method: Option<String>,
    },
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Args::parse().command {
        Command::Migrate { database_url } => Store::connect(&database_url).await?.migrate().await?,
        Command::Provision {
            database_url,
            name,
            principal,
        } => {
            let (project, token) = Store::connect(&database_url)
                .await?
                .provision(&name, &principal)
                .await?;
            println!(
                "{}",
                serde_json::json!({"project_id":project,"token":token})
            );
        }
        Command::Grant {
            database_url,
            project,
            principal,
        } => {
            let token = Store::connect(&database_url)
                .await?
                .grant(&project, &principal)
                .await?;
            println!(
                "{}",
                serde_json::json!({"project_id":project,"token":token})
            );
        }
        Command::Serve { database_url, bind } => {
            let s = Store::connect(&database_url).await?;
            s.check_schema().await?;
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            eprintln!("agent-board listening on {bind}");
            axum::serve(listener, router(s))
                .with_graceful_shutdown(async {
                    shutdown().await;
                })
                .await?;
        }
        Command::Export { url, token, vault } => {
            let written = agent_board::vault::pull(&url, &token, &vault).await?;
            println!("{}", serde_json::json!({"written":written}));
        }
        Command::Request {
            url,
            token,
            path,
            body_file,
            key,
            method,
        } => {
            anyhow::ensure!(path.starts_with("/api/v1/"), "path must start /api/v1/");
            let selected =
                method
                    .as_deref()
                    .unwrap_or(if body_file.is_some() { "POST" } else { "GET" });
            anyhow::ensure!(
                (selected == "GET") == body_file.is_none(),
                "GET takes no body; POST/PATCH require --body-file"
            );
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
            let endpoint = format!("{}{path}", url.trim_end_matches('/'));
            let req = if let Some(file) = body_file {
                let body: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
                client
                    .request(selected.parse::<reqwest::Method>()?, endpoint)
                    .json(&body)
                    .header(
                        "Idempotency-Key",
                        key.ok_or_else(|| anyhow::anyhow!("--key required for writes"))?,
                    )
            } else {
                client.get(endpoint)
            };
            let response = req.bearer_auth(token).send().await?;
            let status = response.status();
            println!("{}", response.text().await?);
            anyhow::ensure!(status.is_success(), "HTTP {status}");
        }
    }
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
