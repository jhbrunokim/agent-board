//! One-way vault projection. Generated files never silently replace user edits.
use anyhow::{Context, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};
use uuid::Uuid;
fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn directory(p: &Path) -> anyhow::Result<()> {
    if let Ok(m) = fs::symlink_metadata(p) {
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "export directory must not be a symlink"
        );
    } else {
        fs::create_dir(p)?;
    }
    Ok(())
}
fn regular(p: &Path) -> anyhow::Result<()> {
    if let Ok(m) = fs::symlink_metadata(p) {
        ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "managed path must be a regular file"
        );
    }
    Ok(())
}
fn atomic(p: &Path, b: &[u8]) -> anyhow::Result<()> {
    regular(p)?;
    let tmp = p.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    use std::io::Write;
    f.write_all(b)?;
    f.sync_all()?;
    fs::rename(tmp, p)?;
    Ok(())
}
struct Lock(std::path::PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
/// Pull current authorized documents. Returns count written; failures leave originals intact.
pub async fn pull(url: &str, token: &str, vault: &Path) -> anyhow::Result<usize> {
    ensure!(vault.is_dir(), "vault must already exist");
    let root = vault.join("AgentBoard");
    directory(&root)?;
    let lock = root.join(".export.lock");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .context("another export is running; stale lock requires manual removal")?;
    let _guard = Lock(lock);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut cursor: Option<String> = None;
    let mut project = String::new();
    let mut manifest = BTreeMap::<String, String>::new();
    let mut count = 0;
    loop {
        let mut request = client
            .get(format!("{}/api/v1/export", url.trim_end_matches('/')))
            .bearer_auth(token)
            .query(&[("limit", "50")]);
        if let Some(c) = &cursor {
            request = request.query(&[("after", c)]);
        }
        let page: Value = request.send().await?.error_for_status()?.json().await?;
        let pid = page["project_id"].as_str().context("missing project_id")?;
        Uuid::parse_str(pid)?;
        if project.is_empty() {
            project = pid.into();
            let dir = root.join(&project);
            directory(&dir)?;
            let mp = dir.join("manifest.json");
            regular(&mp)?;
            if mp.exists() {
                manifest = serde_json::from_slice(&fs::read(mp)?)?;
            }
        }
        ensure!(project == pid, "project scope changed");
        let dir = root.join(&project);
        for doc in page["items"].as_array().context("missing items")? {
            let id = doc["id"].as_str().context("missing id")?;
            let (_, uuid) = id.split_once('-').context("invalid id")?;
            Uuid::parse_str(uuid)?;
            ensure!(
                id.starts_with("object-") || id.starts_with("message-"),
                "invalid type"
            );
            let path = dir.join(format!("{id}.md"));
            regular(&path)?;
            let title = doc["data"]["title"]
                .as_str()
                .unwrap_or(id)
                .replace(['\n', '\r'], " ");
            let body = serde_json::to_string_pretty(&doc["data"])?.replace('<', "&lt;");
            let text = format!(
                "---\nboard_id: {id}\nproject_id: {project}\nsource_version: {}\ngenerated: true\n---\n\n# {}\n\n구조화 원본의 조회용 사본입니다. 개인 메모는 Notes/에 보관하세요.\n\n    {}\n",
                doc["data"]["version"],
                title.replace('<', "&lt;"),
                body.replace('\n', "\n    ")
            );
            if path.exists() {
                let current = fs::read(&path)?;
                let current_hash = hash(&current);
                if current == text.as_bytes() {
                    manifest.insert(id.into(), current_hash);
                    continue;
                }
                ensure!(
                    manifest.get(id) == Some(&current_hash),
                    "user edit conflict: {id}; original preserved"
                );
            }
            atomic(&path, text.as_bytes())?;
            manifest.insert(id.into(), hash(text.as_bytes()));
            atomic(
                &dir.join("manifest.json"),
                &serde_json::to_vec_pretty(&manifest)?,
            )?;
            count += 1;
        }
        if page["has_more"] != true {
            break;
        }
        let next = page["next_cursor"]
            .as_str()
            .context("missing cursor")?
            .to_owned();
        ensure!(
            cursor.as_ref() != Some(&next),
            "export cursor did not advance"
        );
        cursor = Some(next);
    }
    Ok(count)
}
