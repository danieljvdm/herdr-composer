use crate::{
    catalog::{self, Catalog},
    config::Config,
    images::Attachment,
    process, Result, VERSION,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    #[default]
    Worktree,
    Tab,
}
impl LaunchMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "worktree" => Ok(Self::Worktree),
            "tab" => Ok(Self::Tab),
            _ => Err("launch mode must be 'worktree' or 'tab'".into()),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Worktree => "New worktree",
            Self::Tab => "Tab in selected checkout",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Draft {
    pub version: u32,
    pub revision: u64,
    pub task: String,
    pub repo: String,
    pub repo_explicit: bool,
    pub provider: String,
    pub launch_mode: Option<LaunchMode>,
    pub agent: String,
    pub branch: String,
    pub base: String,
    pub model: String,
    pub effort: String,
    pub speed: String,
    pub focus: Option<bool>,
    pub attachments: Vec<Attachment>,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            version: VERSION,
            revision: 0,
            task: String::new(),
            repo: String::new(),
            repo_explicit: false,
            provider: String::new(),
            launch_mode: None,
            agent: String::new(),
            branch: String::new(),
            base: String::new(),
            model: String::new(),
            effort: String::new(),
            speed: String::new(),
            focus: None,
            attachments: vec![],
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderSpec {
    pub id: String,
    pub version: u32,
    pub command: Vec<String>,
    pub cleanup: serde_json::Value,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TaskRequest {
    pub version: u32,
    pub launch_id: String,
    pub task: String,
    pub repository: PathBuf,
    pub common_dir: PathBuf,
    pub invoking_checkout: Option<PathBuf>,
    pub provider: ProviderSpec,
    #[serde(default)]
    pub launch_mode: LaunchMode,
    #[serde(default)]
    pub tab_checkout: Option<PathBuf>,
    pub branch: String,
    pub base_commit: Option<String>,
    pub agent: String,
    pub kind: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub speed: Option<String>,
    pub native_args: Vec<String>,
    pub focus: bool,
    pub attachments: Vec<Attachment>,
    pub diagnostics: Vec<String>,
}
pub fn launch_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub fn checkout(path: &Path) -> Result<PathBuf> {
    Ok(std::fs::canonicalize(process::git(
        path,
        &["rev-parse", "--show-toplevel"],
    )?)?)
}
pub fn common(path: &Path) -> Result<PathBuf> {
    Ok(std::fs::canonicalize(process::git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)?)
}
pub fn primary(path: &Path) -> Result<PathBuf> {
    let list = process::git(path, &["worktree", "list", "--porcelain"])?;
    let p = list
        .lines()
        .next()
        .and_then(|s| s.strip_prefix("worktree "))
        .ok_or("repository has no primary checkout")?;
    Ok(std::fs::canonicalize(p)?)
}
// Only a contiguous prefix is grammar. The remaining bytes are task data.
pub fn directives(text: &str) -> (Draft, String) {
    let mut d = Draft::default();
    let mut rest = text;
    loop {
        let start = rest.trim_start_matches(char::is_whitespace);
        let end = start.find(char::is_whitespace).unwrap_or(start.len());
        let word = &start[..end];
        let target = if word.starts_with('@') && word.len() > 1 {
            Some((&mut d.agent, &word[1..]))
        } else if word.starts_with('>') && word.len() > 1 {
            Some((&mut d.repo, &word[1..]))
        } else if word.starts_with("branch:") && word.len() > 7 {
            Some((&mut d.branch, &word[7..]))
        } else {
            None
        };
        if let Some((field, value)) = target {
            *field = value.into();
            let tail = &start[end..];
            rest = if let Some(tail) = tail.strip_prefix("\r\n") {
                tail
            } else if let Some(c) = tail.chars().next() {
                &tail[c.len_utf8()..]
            } else {
                tail
            };
        } else {
            break;
        }
    }
    (d, rest.into())
}
fn choose(values: &[&str]) -> String {
    values
        .iter()
        .find(|v| !v.is_empty())
        .unwrap_or(&"")
        .to_string()
}
fn optional(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
pub fn resolve(
    d: &Draft,
    c: &Config,
    cat: &Catalog,
    invoking: Option<&Path>,
    state: &Path,
) -> Result<TaskRequest> {
    if d.version != VERSION {
        return Err("unsupported draft version".into());
    }
    let (inline, task) = directives(&d.task);
    if task.trim().is_empty() && d.attachments.is_empty() {
        return Err("Describe the task or attach an image".into());
    }
    let mut diagnostics = cat.diagnostics.clone();
    let mut suggestion = Draft::default();
    if !c.prose_resolver.is_empty() {
        let result = (|| -> Result<Draft> {
            let v=process::run(&c.prose_resolver,Path::new("/"),Some(&serde_json::json!({"version":VERSION,"task":task,"catalog":cat,"repositories":c.repositories})),Duration::from_secs(5))?.json()?;
            if v["version"] != VERSION {
                return Err("unsupported prose resolver version".into());
            }
            Ok(serde_json::from_value(v["suggestions"].clone())?)
        })();
        match result {
            Ok(s) => suggestion = s,
            Err(e) => diagnostics.push(format!("Prose resolver ignored: {e}")),
        }
        // Suggestions are advisory. Validate independently before they can fill a field.
        if (!suggestion.agent.is_empty() || !suggestion.model.is_empty())
            && !cat
                .selection(&suggestion.agent, &suggestion.model, &c.defaults.agent)
                .is_ok_and(|(_, a, _)| process::available(&a.kind))
        {
            diagnostics.push("Prose resolver agent/model suggestion ignored".into());
            suggestion.agent.clear();
            suggestion.model.clear();
            suggestion.effort.clear();
            suggestion.speed.clear();
        }
    }
    let candidates: Vec<_> = c
        .repositories
        .iter()
        .filter_map(|p| checkout(Path::new(p)).ok())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if !suggestion.repo.is_empty()
        && checkout(Path::new(&suggestion.repo)).is_err()
        && candidates
            .iter()
            .filter(|p| p.file_name().is_some_and(|s| s == suggestion.repo.as_str()))
            .count()
            != 1
    {
        diagnostics.push("Prose resolver repository suggestion ignored".into());
        suggestion.repo.clear();
    }
    if !suggestion.provider.is_empty()
        && match suggestion.provider.as_str() {
            "herdr" => false,
            "worktrunk" => !process::available("wt"),
            id => !c
                .providers
                .get(id)
                .and_then(|p| p.command.first())
                .is_some_and(|p| process::available(p)),
        }
    {
        diagnostics.push("Prose resolver provider suggestion ignored".into());
        suggestion.provider.clear();
    }
    let token = choose(&[
        if d.repo_explicit { &d.repo } else { "" },
        &inline.repo,
        &suggestion.repo,
        &c.defaults.repo,
        &d.repo,
    ]);
    let selected = if !token.is_empty() {
        let expanded = if let Some(p) = token.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(p)
        } else {
            PathBuf::from(&token)
        };
        if expanded.exists() {
            checkout(&expanded)?
        } else {
            let hits: std::collections::BTreeSet<_> = candidates
                .iter()
                .filter(|p| p.file_name().is_some_and(|s| s == token.as_str()))
                .cloned()
                .collect();
            if hits.len() != 1 {
                return Err(
                    format!("No unique repository '{token}'; candidates: {candidates:?}").into(),
                );
            }
            hits.into_iter().next().unwrap()
        }
    } else if let Some(p) = invoking {
        checkout(p)?
    } else if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        return Err(format!("Choose a repository; candidates: {candidates:?}").into());
    };
    let common_dir = common(&selected)?;
    let repository = primary(&selected)?;
    let launch_mode = d.launch_mode.unwrap_or(c.defaults.launch_mode);
    if !d.provider.is_empty()
        && !matches!(d.provider.as_str(), "herdr" | "worktrunk")
        && !c.providers.contains_key(&d.provider)
    {
        return Err(format!("unknown provider: {}", d.provider).into());
    }
    if launch_mode == LaunchMode::Tab
        && (!d.branch.is_empty() || !inline.branch.is_empty() || !d.base.is_empty())
    {
        return Err("Tab mode uses the selected checkout as it is. Clear branch/base or choose New worktree.".into());
    }
    let invoking_checkout = invoking
        .and_then(|p| checkout(p).ok())
        .filter(|p| common(p).ok().as_ref() == Some(&common_dir));
    let base_commit = if d.base.is_empty() {
        None
    } else {
        let (dir, base) = if d.base == "@" || d.base == "current" {
            (
                invoking_checkout
                    .as_deref()
                    .ok_or("Current checkout is unavailable for the selected repository")?,
                "HEAD",
            )
        } else {
            (repository.as_path(), d.base.as_str())
        };
        Some(process::git(
            dir,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{base}^{{commit}}"),
            ],
        )?)
    };
    let id = launch_id();
    if !suggestion.branch.is_empty()
        && (suggestion.branch.starts_with('-')
            || process::git(
                &repository,
                &["check-ref-format", "--branch", &suggestion.branch],
            )
            .is_err()
            || process::git(
                &repository,
                &[
                    "show-ref",
                    "--verify",
                    &format!("refs/heads/{}", suggestion.branch),
                ],
            )
            .is_ok())
    {
        diagnostics.push("Prose resolver branch suggestion ignored".into());
        suggestion.branch.clear();
    }
    let branch = if launch_mode == LaunchMode::Tab {
        process::git(&selected, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_default()
    } else {
        let branch = choose(&[&d.branch, &inline.branch, &suggestion.branch]);
        let branch = if branch.is_empty() {
            format!("task-{}", id)
        } else {
            branch
        };
        if branch.starts_with('-') {
            return Err("branch cannot start with '-'".into());
        }
        process::git(&repository, &["check-ref-format", "--branch", &branch])?;
        if process::git(
            &repository,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")],
        )
        .is_ok()
        {
            return Err(format!("Branch '{branch}' already exists; choose a new branch or use tab mode in its checkout.").into());
        }
        branch
    };
    let spec = if launch_mode == LaunchMode::Tab {
        ProviderSpec {
            id: "herdr".into(),
            version: VERSION,
            command: vec![],
            cleanup: serde_json::Value::Null,
        }
    } else {
        let provider = choose(&[&d.provider, &suggestion.provider, &c.defaults.workspace]);
        let spec = match provider.as_str() {
            "herdr" | "worktrunk" => ProviderSpec {
                id: provider.clone(),
                version: VERSION,
                command: vec![],
                cleanup: serde_json::Value::Null,
            },
            _ => {
                let p = c
                    .providers
                    .get(&provider)
                    .ok_or_else(|| format!("unknown provider: {provider}"))?;
                ProviderSpec {
                    id: provider.clone(),
                    version: VERSION,
                    command: p.command.clone(),
                    cleanup: p.cleanup.clone(),
                }
            }
        };
        if provider == "worktrunk" && !process::available("wt") {
            return Err("Worktrunk provider requires wt".into());
        }
        if provider != "worktrunk"
            && provider != "herdr"
            && !spec.command.first().is_some_and(|s| process::available(s))
        {
            return Err("configured provider command is unavailable".into());
        }
        spec
    };
    let agent = choose(&[&d.agent, &inline.agent, &suggestion.agent]);
    if !suggestion.model.is_empty()
        && !cat
            .selection(&agent, &suggestion.model, &c.defaults.agent)
            .is_ok_and(|(_, a, m)| {
                catalog::native_args(&a.kind, m.as_ref().map(|m| m.id.as_str()), None, None).is_ok()
            })
    {
        diagnostics.push("Prose resolver model suggestion ignored".into());
        suggestion.model.clear();
    }
    let model = choose(&[&d.model, &suggestion.model, &c.defaults.model]);
    let (agent, a, m) = cat.selection(&agent, &model, &c.defaults.agent)?;
    for (name, value, supported) in [
        (
            "effort",
            &mut suggestion.effort,
            m.as_ref().map(|m| &m.efforts),
        ),
        (
            "speed",
            &mut suggestion.speed,
            m.as_ref().map(|m| &m.speeds),
        ),
    ] {
        let adapter_supported = if name == "effort" {
            catalog::native_args(&a.kind, None, Some(value), None)
        } else {
            catalog::native_args(&a.kind, None, None, Some(value))
        }
        .is_ok();
        if !value.is_empty()
            && (!supported.is_some_and(|s| s.contains(value)) || !adapter_supported)
        {
            diagnostics.push(format!("Prose resolver {name} suggestion ignored"));
            value.clear();
        }
    }
    let effort = optional(choose(&[
        &d.effort,
        &suggestion.effort,
        m.as_ref().map_or("", |m| m.default_effort.as_str()),
        &c.defaults.effort,
    ]));
    let speed = optional(choose(&[
        &d.speed,
        &suggestion.speed,
        m.as_ref().map_or("", |m| m.default_speed.as_str()),
        &c.defaults.speed,
    ]));
    for (name, value, supported) in [
        ("effort", &effort, m.as_ref().map(|m| &m.efforts)),
        ("speed", &speed, m.as_ref().map(|m| &m.speeds)),
    ] {
        if let Some(value) = value {
            if !supported.is_some_and(|s| s.contains(value)) {
                return Err(format!(
                    "Unsupported {name} '{value}' for selected model; correct the saved choice"
                )
                .into());
            }
        }
    }
    let model = m.map(|m| m.id);
    let native_args = catalog::native_args(
        &a.kind,
        model.as_deref(),
        effort.as_deref(),
        speed.as_deref(),
    )?;
    if !process::available(&a.kind) {
        return Err(format!("agent executable is unavailable: {}", a.kind).into());
    }
    let attachments = d
        .attachments
        .iter()
        .map(|a| {
            crate::images::import_file(Path::new(&a.path), &state.join("attachments"))
                .map(|(mut kept, _)| {
                    if !a.name.is_empty() {
                        kept.name = a.name.clone();
                    }
                    kept
                })
                .map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(TaskRequest {
        version: VERSION,
        launch_id: id,
        task,
        repository,
        common_dir,
        invoking_checkout,
        provider: spec,
        launch_mode,
        tab_checkout: (launch_mode == LaunchMode::Tab).then_some(selected),
        branch,
        base_commit,
        agent,
        kind: a.kind,
        model,
        effort,
        speed,
        native_args,
        focus: d.focus.or(suggestion.focus).unwrap_or(c.defaults.focus),
        attachments,
        diagnostics,
    })
}
