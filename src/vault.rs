//! One-way, resumable vault projection. Generated files never silently replace user edits.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};
use uuid::Uuid;

const RENDERER_VERSION: u32 = 2;
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
type Manifest = BTreeMap<String, String>;

fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn directory(p: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) => ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "export directory must not be a symlink"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(p)?,
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
fn regular(p: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) => ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "managed path must be a regular file"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
fn atomic(p: &Path, b: &[u8]) -> anyhow::Result<()> {
    regular(p)?;
    let tmp = p.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        use std::io::Write;
        f.write_all(b)?;
        f.sync_all()?;
        fs::rename(&tmp, p)?;
        fs::File::open(p.parent().context("missing parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
fn save<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    atomic(path, &serde_json::to_vec_pretty(value)?)
}
fn uuid(value: &str) -> anyhow::Result<()> {
    ensure!(
        Uuid::parse_str(value)?.to_string() == value,
        "invalid canonical identifier"
    );
    Ok(())
}
fn document_id(id: &str, legacy: bool) -> anyhow::Result<()> {
    let (kind, value) = id.split_once('-').context("invalid document identifier")?;
    ensure!(
        ["object", "thread", "source", "review"].contains(&kind) || (legacy && kind == "message"),
        "invalid document kind"
    );
    uuid(value)
}
#[derive(Serialize, Deserialize, Default)]
struct SyncState {
    server_url: String,
    project_id: String,
    renderer_version: u32,
    cursor: Option<String>,
    #[serde(default)]
    pending: Manifest,
}
struct Lock(std::path::PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// User strings are always indented text; headings additionally escape Markdown.
fn plain(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn heading(value: &str) -> String {
    let mut out = String::new();
    for c in plain(value).replace(['\n', '\r', '\t'], " ").chars() {
        if "\\`*_{}[]()#+-.!|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn text(out: &mut String, value: &str) {
    for line in plain(value).lines() {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
}
fn field(out: &mut String, name: &str, value: &Value) {
    if !value.is_null() {
        out.push_str(&format!("**{name}**\n\n"));
        if let Some(value) = value.as_str() {
            text(out, value);
        } else {
            text(
                out,
                &serde_json::to_string_pretty(value).unwrap_or_default(),
            );
        }
    }
}
fn link(project: &str, kind: &str, id: &Value) -> Option<String> {
    let id = id.as_str()?;
    uuid(id).ok()?;
    Some(format!("[[AgentBoard/{project}/{kind}-{id}]]"))
}
fn render(project: &str, item: &Value) -> anyhow::Result<(String, String)> {
    let id = item["id"].as_str().context("missing id")?;
    document_id(id, false)?;
    let kind = item["kind"].as_str().context("missing kind")?;
    let prefix = match kind {
        "task" | "knowledge" => "object",
        "thread" => "thread",
        "source" => "source",
        "review" => "review",
        _ => anyhow::bail!("unknown document kind"),
    };
    ensure!(
        id.starts_with(&format!("{prefix}-")),
        "document kind mismatch"
    );
    let d = &item["data"];
    ensure!(d.is_object(), "missing document data");
    if let Some(pid) = d["project_id"].as_str() {
        ensure!(pid == project, "document project mismatch");
    }
    let title = d["title"]
        .as_str()
        .or_else(|| d["label"].as_str())
        .unwrap_or(id);
    let mut out = format!(
        "---\nboard_id: {id}\nproject_id: {project}\nkind: {kind}\nversion: {}\nsource_version: {}\nstatus: {}\nrenderer_version: {RENDERER_VERSION}\ngenerated: true\n---\n\n# {}\n\n",
        d["version"],
        d["version"],
        d["status"],
        heading(title)
    );
    field(&mut out, "ID", &Value::String(id.into()));
    field(&mut out, "Kind", &item["kind"]);
    field(&mut out, "Version", &d["version"]);
    field(&mut out, "Status", &d["status"]);
    field(&mut out, "Content revision", &d["content_revision"]);
    match kind {
        "task" => {
            out.push_str("## Description\n\n");
            field(&mut out, "Description", &d["description"]);
            out.push_str("## Lease\n\n");
            match d.get("lease") {
                Some(Value::Null) => out.push_str("No active lease.\n\n"),
                Some(lease) => field(&mut out, "Lease", lease),
                None => out.push_str("Lease details omitted from this projection.\n\n"),
            }
            out.push_str("## Pinned knowledge\n\nWikilinks open the latest generated knowledge note. The pinned version and lookup below identify the exact historical content used by this task.\n\n");
            if let Some(links) = d["knowledge_links"].as_array() {
                for entry in links {
                    if let Some(target) = link(project, "object", &entry["knowledge_id"]) {
                        out.push_str(&format!("- {target}\n\n"));
                    }
                    field(
                        &mut out,
                        "Pinned content revision",
                        &entry["knowledge_content_revision"],
                    );
                    field(&mut out, "Pinned version", &entry["knowledge_version"]);
                    field(&mut out, "Pinned lookup", &entry["lookup"]);
                }
            }
        }
        "knowledge" => {
            out.push_str("## Body\n\n");
            if let Some(body) = d["body"].as_str() {
                text(&mut out, body);
            }
            out.push_str("## Sources\n\n");
            if let Some(sources) = d["sources"].as_array() {
                for source in sources {
                    if let Some(target) = link(project, "source", &source["id"]) {
                        out.push_str(&format!("- {target}\n\n"));
                    }
                    field(&mut out, "Label", &source["label"]);
                    field(&mut out, "Revision", &source["revision"]);
                }
            }
        }
        "thread" => {
            out.push_str("## Replies\n\n");
            if let Some(messages) = d["messages"].as_array() {
                for message in messages {
                    let sender = message["sender"].as_str().unwrap_or("Unknown sender");
                    let mid = message["id"].as_str().unwrap_or("unknown");
                    out.push_str(&format!("### {} — {}\n\n", heading(sender), heading(mid)));
                    field(&mut out, "Sequence", &message["seq"]);
                    // Arbitrary protocol payloads remain complete, pretty, inert JSON.
                    field(&mut out, "Payload", &message["payload"]);
                }
            }
        }
        "source" => {
            field(&mut out, "Label", &d["label"]);
            field(&mut out, "URI (reference only)", &d["uri"]);
            field(&mut out, "Revision", &d["revision"]);
        }
        "review" => {
            field(&mut out, "Reviewer", &d["reviewer"]);
            field(&mut out, "Verdict", &d["status"]);
            field(&mut out, "Subject content revision", &d["subject_revision"]);
            if let Some(target) = link(project, "object", &d["subject_id"]) {
                out.push_str(&format!("## Subject\n\n{target}\n\n"));
            }
            field(&mut out, "Comment", &d["comment"]);
        }
        _ => unreachable!(),
    }
    if item["incomplete"] == true || d["incomplete"] == true || d["truncated"] == true {
        out.push_str("## Incomplete projection\n\nThis bounded projection omits some content. Retrieve the original content using the lookup below.\n\n");
        if kind == "thread" {
            out.push_str("For earlier replies, start at the lookup without an after cursor. The continuation cursor retrieves newer replies after this preview.\n\n");
        }
        field(
            &mut out,
            "Earlier replies omitted",
            &d["earlier_messages_omitted"],
        );
        field(
            &mut out,
            "Lookup",
            item.get("lookup").unwrap_or(&d["lookup"]),
        );
        field(&mut out, "Continuation cursor", &d["next_cursor"]);
    }
    Ok((id.into(), out))
}

/// Pull authorized projections, returning the number of Markdown documents changed.
/// State is committed after each fully written page; retries recover interrupted writes.
pub async fn pull(url: &str, token: &str, vault: &Path) -> anyhow::Result<usize> {
    ensure!(vault.is_dir(), "vault must already exist");
    directory(vault)?;
    let root = vault.join("AgentBoard");
    directory(&root)?;
    let lock = root.join(".export.lock");
    regular(&lock)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .context("another export is running; stale lock requires manual removal")?;
    let _guard = Lock(lock);
    let server_url = url.trim_end_matches('/').to_owned();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    // Resolve authorization scope before opening any project-specific local state.
    let caps: Value = client
        .get(format!("{server_url}/api/v1/capabilities"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project = caps["project_id"].as_str().context("missing project_id")?;
    uuid(project)?;
    let dir = root.join(project);
    directory(&dir)?;
    let mp = dir.join("manifest.json");
    let sp = dir.join("sync-state.json");
    regular(&mp)?;
    regular(&sp)?;
    let mut manifest: Manifest = if mp.exists() {
        serde_json::from_slice(&fs::read(&mp)?)?
    } else {
        Manifest::new()
    };
    let mut state: SyncState = if sp.exists() {
        serde_json::from_slice(&fs::read(&sp)?)?
    } else {
        SyncState::default()
    };
    // Verify even unchanged notes: an empty delta must still detect user edits.
    for (id, expected) in manifest.iter().chain(state.pending.iter()) {
        document_id(id, true)?;
        let path = dir.join(format!("{id}.md"));
        regular(&path)?;
        if !path.exists() {
            ensure!(!manifest.contains_key(id), "managed file missing: {id}");
            continue;
        }
        let actual = hash(&fs::read(&path)?);
        ensure!(
            manifest.get(id) == Some(&actual) || state.pending.get(id) == Some(&actual),
            "user edit conflict: {id}; original preserved"
        );
        // A journaled generated file may have reached disk just before a crash.
        let _ = expected;
    }
    for (id, expected) in &state.pending {
        let path = dir.join(format!("{id}.md"));
        if path.exists() && hash(&fs::read(path)?) == *expected {
            manifest.insert(id.clone(), expected.clone());
        }
    }
    if !state.pending.is_empty() {
        save(&mp, &manifest)?;
    }
    if state.server_url != server_url
        || state.project_id != project
        || state.renderer_version != RENDERER_VERSION
    {
        state = SyncState {
            server_url,
            project_id: project.into(),
            renderer_version: RENDERER_VERSION,
            ..Default::default()
        };
    }
    let mut count = 0;
    loop {
        let mut request = client
            .get(format!("{}/api/v1/vault", state.server_url))
            .bearer_auth(token)
            .query(&[("limit", "50")]);
        if let Some(cursor) = &state.cursor {
            request = request.query(&[("after", cursor)]);
        }
        let mut response = request.send().await?.error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_PAGE_BYTES as u64),
            "vault page too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_PAGE_BYTES,
                "vault page too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        let page: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            page["project_id"].as_str() == Some(project),
            "project scope changed"
        );
        ensure!(
            page["renderer_version"].as_u64() == Some(RENDERER_VERSION as u64),
            "unsupported vault renderer version"
        );
        let next = page["next_cursor"]
            .as_str()
            .context("missing cursor")?
            .to_owned();
        let more = page["has_more"].as_bool().context("missing has_more")?;
        ensure!(
            !more || state.cursor.as_ref() != Some(&next),
            "vault cursor did not advance"
        );
        let items = page["items"].as_array().context("missing items")?;
        ensure!(items.len() <= 150, "too many vault items");
        let mut docs = BTreeMap::new();
        for item in items {
            let (id, rendered) = render(project, item)?;
            ensure!(
                docs.insert(id, rendered).is_none(),
                "duplicate vault document"
            );
        }
        // Check the whole page before writing its journal or any content.
        for (id, rendered) in &docs {
            let path = dir.join(format!("{id}.md"));
            regular(&path)?;
            if path.exists() {
                let current = fs::read(&path)?;
                ensure!(
                    current == rendered.as_bytes() || manifest.get(id) == Some(&hash(&current)),
                    "user edit conflict: {id}; original preserved"
                );
            }
        }
        state.pending = docs
            .iter()
            .map(|(id, rendered)| (id.clone(), hash(rendered.as_bytes())))
            .collect();
        save(&sp, &state)?;
        for (id, rendered) in docs {
            let path = dir.join(format!("{id}.md"));
            regular(&path)?;
            let current = if path.exists() {
                Some(fs::read(&path)?)
            } else {
                None
            };
            if let Some(current) = &current {
                ensure!(
                    current == rendered.as_bytes() || manifest.get(&id) == Some(&hash(current)),
                    "user edit conflict: {id}; original preserved"
                );
            }
            let identical = current.as_deref() == Some(rendered.as_bytes());
            if !identical {
                atomic(&path, rendered.as_bytes())?;
                count += 1;
            }
            manifest.insert(id, hash(rendered.as_bytes()));
            save(&mp, &manifest)?;
        }
        state.cursor = Some(next);
        state.pending.clear();
        save(&sp, &state)?;
        if !more {
            return Ok(count);
        }
    }
}
