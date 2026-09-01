use crate::{
    config::Paths,
    process,
    request::{self, LaunchMode, TaskRequest},
    storage, Result, VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Herdr {
    pub binary: String,
    pub socket: String,
}
impl Herdr {
    pub fn current() -> Result<Self> {
        Ok(Self {
            binary: env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
            socket: env::var("HERDR_SOCKET_PATH").map_err(|_| {
                "Run Composer in a Herdr pane, or set HERDR_SOCKET_PATH to the intended session"
            })?,
        })
    }
    pub fn output(&self, args: &[&str]) -> Result<process::Output> {
        let mut argv = vec![
            "/usr/bin/env".into(),
            format!("HERDR_SOCKET_PATH={}", self.socket),
            self.binary.clone(),
        ];
        argv.extend(args.iter().map(|s| s.to_string()));
        process::run(&argv, Path::new("/"), None, Duration::from_secs(330))
    }
    pub fn call(&self, args: &[&str]) -> Result<Value> {
        let output = self.output(args)?.checked()?;
        if output.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&output)?)
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub launch_id: String,
    pub checkout: PathBuf,
    pub branch: String,
    #[serde(default)]
    pub tab: Option<String>,
    pub owned: bool,
    pub workspace: Option<String>,
    pub pane: Option<String>,
    pub prepared_head: Option<String>,
    pub cleanup: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Delivery {
    NotSent,
    Unknown,
    Confirmed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub version: u32,
    pub id: String,
    pub request: Option<TaskRequest>,
    pub herdr: Herdr,
    pub source_workspace: Option<String>,
    pub runner_pane: Option<String>,
    #[serde(default)]
    pub cleanup_pane: Option<String>,
    pub receipt: Option<Receipt>,
    pub step: String,
    pub error: Option<String>,
    pub delivery: Delivery,
    pub agent: Option<Value>,
    pub prompt_result: Option<Value>,
    pub draft: Option<(PathBuf, u64)>,
    pub removal: Option<Value>,
}
pub fn path(state: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("invalid session ID".into());
    }
    Ok(state.join("sessions").join(format!("{id}.json")))
}
pub fn load(state: &Path, id: &str) -> Result<SessionRecord> {
    let r: SessionRecord = storage::read_json(&path(state, id)?)?;
    if r.version != VERSION {
        return Err("unsupported session version".into());
    }
    if r.id != id {
        return Err("session ID mismatch".into());
    }
    Ok(r)
}
fn save(state: &Path, r: &SessionRecord) -> Result<()> {
    storage::write_json(&path(state, &r.id)?, r)
}
fn field(v: &Value, pointer: &str) -> Result<String> {
    Ok(v.pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Herdr response missing {pointer}: {v}"))?
        .into())
}
pub fn preflight(h: &Herdr) -> Result<()> {
    h.call(&["workspace", "list"])?;
    Ok(())
}

pub fn submit(
    paths: &Paths,
    request: TaskRequest,
    draft: Option<(PathBuf, u64)>,
) -> Result<String> {
    let h = Herdr::current()?;
    preflight(&h)?;
    let mut r = SessionRecord {
        version: VERSION,
        id: request.launch_id.clone(),
        request: Some(request),
        herdr: h,
        source_workspace: None,
        runner_pane: None,
        cleanup_pane: None,
        receipt: None,
        step: "submitted".into(),
        error: None,
        delivery: Delivery::NotSent,
        agent: None,
        prompt_result: None,
        draft,
        removal: None,
    };
    let mut submission_lock = Some(storage::lock(
        &path(&paths.state, &r.id)?.with_extension("lock"),
    )?);
    if path(&paths.state, &r.id)?.exists() {
        return Err("session already submitted; no launch replay".into());
    }
    save(&paths.state, &r)?;
    let result = (|| -> Result<()> {
        let req = r.request.as_ref().unwrap();
        let source_checkout = if req.launch_mode == LaunchMode::Tab {
            req.tab_checkout
                .as_ref()
                .ok_or("missing selected checkout")?
        } else {
            &req.repository
        };
        let repo = source_checkout.to_str().ok_or("non-UTF8 repository")?;
        let list = r.herdr.call(&["workspace", "list"])?;
        let source = list
            .pointer("/result/workspaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|w| {
                w.pointer("/worktree/checkout_path")
                    .and_then(Value::as_str)
                    .and_then(|p| fs::canonicalize(p).ok())
                    .as_ref()
                    == Some(source_checkout)
            })
            .and_then(|w| w["workspace_id"].as_str())
            .map(String::from);
        let source = if source.is_some() {
            source
        } else {
            let panes = r.herdr.call(&["pane", "list"])?;
            panes
                .pointer("/result/panes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|p| {
                    p["cwd"]
                        .as_str()
                        .and_then(|p| fs::canonicalize(p).ok())
                        .as_ref()
                        == Some(source_checkout)
                })
                .and_then(|p| p["workspace_id"].as_str())
                .map(String::from)
        };
        let source = match source {
            Some(s) => s,
            None => {
                r.step = "creating_source".into();
                save(&paths.state, &r)?;
                let created = r.herdr.call(&[
                    "workspace",
                    "create",
                    "--cwd",
                    repo,
                    "--label",
                    "Composer source",
                    "--no-focus",
                ])?;
                field(&created, "/result/workspace/workspace_id")?
            }
        };
        r.source_workspace = Some(source.clone());
        r.step = "source_ready".into();
        save(&paths.state, &r)?;
        let tab = r.herdr.call(&[
            "tab",
            "create",
            "--workspace",
            &source,
            "--cwd",
            repo,
            "--label",
            "Composer preparing",
            "--no-focus",
        ])?;
        let pane = field(&tab, "/result/root_pane/pane_id")?;
        r.runner_pane = Some(pane.clone());
        r.step = "queued".into();
        save(&paths.state, &r)?;
        let exe = env::current_exe()?;
        // The runner gets a frozen session ID. Prompt bytes never enter a shell.
        let command = format!(
            "{} __run {}",
            process::quote(&exe.to_string_lossy()),
            process::quote(&r.id)
        );
        let command = format!(
            "COMPOSER_STATE_DIR={} HERDR_PLUGIN_STATE_DIR={} {}",
            process::quote(&paths.state.to_string_lossy()),
            process::quote(&paths.state.to_string_lossy()),
            command
        );
        drop(submission_lock.take());
        r.herdr
            .output(&["pane", "run", &pane, &command])?
            .checked()?;
        Ok(())
    })();
    if let Err(e) = result {
        // A lost pane-run response may already have started the runner. Never
        // overwrite its checkpoints with the submitter's stale copy.
        if submission_lock.is_some() {
            r.error = Some(e.to_string());
            save(&paths.state, &r)?;
        } else if let Ok(_lock) = storage::lock(&path(&paths.state, &r.id)?.with_extension("lock"))
        {
            let mut live = load(&paths.state, &r.id)?;
            if live.step == "queued" {
                live.error = Some(format!("runner handoff uncertain: {e}"));
                save(&paths.state, &live)?;
            }
        }
        return Err(format!(
            "Session {} needs attention: {e}. Inspect {}",
            r.id,
            path(&paths.state, &r.id)?.display()
        )
        .into());
    }
    Ok(r.id)
}
fn validate_receipt(req: &TaskRequest, receipt: &Receipt) -> Result<()> {
    if req.launch_mode == LaunchMode::Tab {
        if receipt.version != VERSION
            || receipt.launch_id != req.launch_id
            || receipt.owned
            || receipt.tab.is_none()
        {
            return Err("invalid tab ownership receipt".into());
        }
        if Some(&receipt.checkout) != req.tab_checkout.as_ref()
            || fs::canonicalize(&receipt.checkout)? != receipt.checkout
            || request::common(&receipt.checkout)? != req.common_dir
        {
            return Err("tab checkout identity changed".into());
        }
        return Ok(());
    }
    if receipt.version != VERSION || receipt.launch_id != req.launch_id || !receipt.owned {
        return Err(
            "provider receipt has no valid launch ownership; inspect resources manually".into(),
        );
    }
    if !receipt.checkout.is_absolute() || fs::canonicalize(&receipt.checkout)? != receipt.checkout {
        return Err("provider checkout is not canonical".into());
    }
    if receipt.checkout == req.repository
        || request::primary(&receipt.checkout)? == receipt.checkout
    {
        return Err("refusing ownership of primary checkout".into());
    }
    if request::common(&receipt.checkout)? != req.common_dir {
        return Err("provider checkout belongs to a different repository".into());
    }
    if receipt.branch != req.branch
        || process::git(&receipt.checkout, &["symbolic-ref", "--short", "HEAD"])? != req.branch
    {
        return Err("provider branch differs from requested branch".into());
    }
    Ok(())
}
fn opened(r: &mut SessionRecord, v: &Value) -> Result<()> {
    let receipt = r.receipt.as_mut().ok_or("missing checkout receipt")?;
    receipt.workspace = Some(field(v, "/result/workspace/workspace_id")?);
    receipt.pane = Some(field(v, "/result/root_pane/pane_id")?);
    Ok(())
}
fn prepare(state: &Path, r: &mut SessionRecord) -> Result<()> {
    let req = r.request.as_ref().unwrap().clone();
    if req.version != VERSION || req.provider.version != VERSION {
        return Err("unsupported request/provider version".into());
    }
    let source = r
        .source_workspace
        .clone()
        .ok_or("missing pinned source workspace")?;
    let binding = r
        .herdr
        .call(&["worktree", "list", "--workspace", &source])?;
    if fs::canonicalize(field(&binding, "/result/source/repo_root")?)? != req.repository {
        return Err("source workspace repository binding changed before preparation".into());
    }
    r.step = "preparing".into();
    save(state, r)?;
    if req.launch_mode == LaunchMode::Tab {
        let checkout = req
            .tab_checkout
            .as_ref()
            .ok_or("tab request has no selected checkout")?;
        if request::checkout(checkout)? != *checkout || request::common(checkout)? != req.common_dir
        {
            return Err("selected checkout changed before tab preparation".into());
        }
        let v = r.herdr.call(&[
            "tab",
            "create",
            "--workspace",
            &source,
            "--cwd",
            checkout.to_str().ok_or("non-UTF8 checkout")?,
            "--label",
            "Composer task",
            "--no-focus",
        ])?;
        r.receipt = Some(Receipt {
            version: VERSION,
            launch_id: r.id.clone(),
            checkout: checkout.clone(),
            branch: req.branch.clone(),
            owned: false,
            tab: Some(field(&v, "/result/tab/tab_id")?),
            workspace: Some(source),
            pane: Some(field(&v, "/result/root_pane/pane_id")?),
            prepared_head: None,
            cleanup: Value::Null,
        });
        r.step = "tab_created".into();
        save(state, r)?;
    } else {
        match req.provider.id.as_str() {
            "herdr" => {
                let mut args = vec![
                    "worktree",
                    "create",
                    "--workspace",
                    &source,
                    "--branch",
                    &req.branch,
                    "--no-focus",
                ];
                if let Some(base) = &req.base_commit {
                    args.extend(["--base", base]);
                }
                let v = r.herdr.call(&args)?;
                let checkout = fs::canonicalize(field(&v, "/result/worktree/path")?)?;
                r.receipt = Some(Receipt {
                    version: VERSION,
                    launch_id: r.id.clone(),
                    checkout,
                    branch: req.branch.clone(),
                    tab: None,
                    owned: true,
                    workspace: None,
                    pane: None,
                    prepared_head: None,
                    cleanup: Value::Null,
                });
                opened(r, &v)?;
                save(state, r)?;
            }
            "worktrunk" => {
                let mut args = vec![
                    "wt".into(),
                    "switch".into(),
                    "--create".into(),
                    req.branch.clone(),
                    "--no-cd".into(),
                    "--format=json".into(),
                ];
                if let Some(base) = &req.base_commit {
                    args.extend(["--base".into(), base.clone()]);
                }
                let output = process::run(&args, &req.repository, None, Duration::from_secs(300))?;
                if !output.stderr.is_empty() {
                    eprintln!("{}", output.stderr);
                }
                // Even a hook failure may return a receipt. Persist it before reporting
                // failure; never discover ownership by guessing a path from a branch.
                let v: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);
                if let Some(p) = v["path"].as_str() {
                    let receipt = Receipt {
                        version: VERSION,
                        launch_id: r.id.clone(),
                        checkout: fs::canonicalize(p)?,
                        branch: req.branch.clone(),
                        tab: None,
                        owned: true,
                        workspace: None,
                        pane: None,
                        prepared_head: None,
                        cleanup: Value::Null,
                    };
                    validate_receipt(&req, &receipt)?;
                    r.receipt = Some(receipt);
                    r.step = "checkout_created".into();
                    save(state, r)?;
                }
                output.checked()?;
                let receipt = r.receipt.as_ref().ok_or(
                    "Worktrunk returned no valid checkout receipt; inspect partial resources",
                )?;
                let v = r.herdr.call(&[
                    "worktree",
                    "open",
                    "--workspace",
                    &source,
                    "--path",
                    receipt.checkout.to_str().ok_or("non-UTF8 checkout")?,
                    "--no-focus",
                ])?;
                opened(r, &v)?;
                save(state, r)?;
            }
            _ => {
                let input = json!({"version":VERSION,"operation":"prepare","launch_id":r.id,"workspace":{"repository":req.repository,"common_dir":req.common_dir,"branch":req.branch,"base_commit":req.base_commit,"source_workspace":source},"cleanup":req.provider.cleanup});
                let output = process::run(
                    &req.provider.command,
                    &req.repository,
                    Some(&input),
                    Duration::from_secs(300),
                )?;
                if !output.stderr.is_empty() {
                    eprintln!("{}", output.stderr);
                }
                let v: Value = serde_json::from_str(&output.stdout)?;
                if v["version"] != VERSION {
                    return Err("unsupported provider response version".into());
                }
                if let Some(value) = v.get("receipt").filter(|v| !v.is_null()) {
                    let receipt: Receipt = serde_json::from_value(value.clone())?;
                    validate_receipt(&req, &receipt)?;
                    r.receipt = Some(receipt);
                    r.step = "checkout_created".into();
                    save(state, r)?;
                }
                output.checked()?;
                if v["status"] != "prepared" {
                    return Err(format!("provider did not finish preparation: {v}").into());
                }
                let receipt = r.receipt.as_ref().ok_or("provider returned no receipt")?;
                if receipt.workspace.is_none() {
                    let v = r.herdr.call(&[
                        "worktree",
                        "open",
                        "--workspace",
                        &source,
                        "--path",
                        receipt.checkout.to_str().ok_or("non-UTF8 checkout")?,
                        "--no-focus",
                    ])?;
                    opened(r, &v)?;
                    save(state, r)?;
                }
            }
        }
    }
    let receipt = r.receipt.as_mut().ok_or("missing prepared workspace")?;
    validate_receipt(&req, receipt)?;
    receipt.prepared_head = Some(process::git(&receipt.checkout, &["rev-parse", "HEAD"])?);
    validate_binding(r)?;
    let receipt = r.receipt.as_ref().unwrap();
    let workspace = receipt
        .workspace
        .as_deref()
        .ok_or("prepared receipt has no workspace")?;
    let panes = r.herdr.call(&["pane", "list", "--workspace", workspace])?;
    let pane = panes
        .pointer("/result/panes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|p| p["pane_id"].as_str() == receipt.pane.as_deref())
        .ok_or("prepared pane does not belong to its recorded workspace")?;
    if pane["cwd"]
        .as_str()
        .and_then(|p| fs::canonicalize(p).ok())
        .as_ref()
        != Some(&receipt.checkout)
    {
        return Err("prepared pane checkout changed".into());
    }
    r.step = "prepared".into();
    save(state, r)
}
fn validate_binding(r: &SessionRecord) -> Result<()> {
    let req = r.request.as_ref().ok_or("session was removed")?;
    let receipt = r.receipt.as_ref().ok_or("missing ownership receipt")?;
    if req.launch_mode == LaunchMode::Tab {
        let workspace = receipt
            .workspace
            .as_deref()
            .ok_or("tab has no recorded workspace")?;
        let tab = receipt.tab.as_deref().ok_or("tab has no recorded ID")?;
        let tabs = r.herdr.call(&["tab", "list", "--workspace", workspace])?;
        if !tabs
            .pointer("/result/tabs")
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|t| t["tab_id"] == tab))
        {
            return Err("recorded tab is missing or moved; refusing to target another tab".into());
        }
        let panes = r.herdr.call(&["pane", "list", "--workspace", workspace])?;
        let pane = panes
            .pointer("/result/panes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|p| p["pane_id"].as_str() == receipt.pane.as_deref())
            .ok_or("recorded tab pane is missing")?;
        if pane["tab_id"] != tab
            || pane["workspace_id"] != workspace
            || pane["cwd"]
                .as_str()
                .and_then(|p| fs::canonicalize(p).ok())
                .as_ref()
                != Some(&receipt.checkout)
        {
            return Err("recorded tab/pane binding changed; refusing cleanup".into());
        }
        if receipt.pane == r.runner_pane || receipt.pane == r.cleanup_pane {
            return Err("refusing to close the Composer runner".into());
        }
        return Ok(());
    }
    let list = r.herdr.call(&["workspace", "list"])?;
    let all = list
        .pointer("/result/workspaces")
        .and_then(Value::as_array)
        .ok_or("missing workspace list")?;
    if let Some(id) = &receipt.workspace {
        if Some(id) == r.source_workspace.as_ref() {
            return Err("refusing to close source workspace".into());
        }
        let workspace = all
            .iter()
            .find(|w| w["workspace_id"] == *id)
            .ok_or("recorded workspace is no longer open; inspect before cleanup")?;
        let p = workspace
            .pointer("/worktree/checkout_path")
            .and_then(Value::as_str)
            .ok_or("workspace has no checkout binding")?;
        if Path::new(p) != receipt.checkout {
            return Err("workspace checkout binding changed; refusing cleanup".into());
        }
        let root = workspace
            .pointer("/worktree/repo_root")
            .and_then(Value::as_str)
            .ok_or("workspace has no repository binding")?;
        if fs::canonicalize(root)? != req.repository {
            return Err("workspace repository binding changed".into());
        }
    } else if all.iter().any(|w| {
        w.pointer("/worktree/checkout_path")
            .and_then(Value::as_str)
            .is_some_and(|p| Path::new(p) == receipt.checkout)
    }) {
        return Err(
            "checkout was opened outside this receipt; inspect and reconcile workspace ownership"
                .into(),
        );
    }
    Ok(())
}
fn name_branch(state: &Path, record: &mut SessionRecord) -> Result<()> {
    let request = record.request.as_ref().ok_or("missing task request")?;
    let Some(config) = request.branch_naming.clone() else {
        return Ok(());
    };
    let task = request.task.clone();
    // Persist the attempt before calling the model. An interrupted runner must
    // not silently replay a paid naming call or change a prepared branch.
    record.step = "naming_branch".into();
    save(state, record)?;
    let result = crate::branch_name::generate(&config, &task);
    let request = record.request.as_mut().unwrap();
    match result {
        Ok(name) => {
            if process::git(
                &request.repository,
                &["show-ref", "--verify", &format!("refs/heads/{name}")],
            )
            .is_ok()
            {
                request
                    .diagnostics
                    .push("Generated branch already exists; using a unique task name".into());
            } else {
                request.branch = name;
            }
        }
        Err(e) => request.diagnostics.push(format!(
            "Branch naming failed: {e}; using a unique task name"
        )),
    }
    record.step = "branch_named".into();
    save(state, record)
}

fn wait_for_codex_input(h: &Herdr, name: &str, pane: &str, deadline: Instant) -> Result<()> {
    let mut reported_blocker = false;
    loop {
        let live = h.call(&["agent", "get", name])?;
        let agent = &live["result"]["agent"];
        if agent["pane_id"] != pane || agent["agent"] != "codex" || agent["name"] != name {
            return Err("Codex startup identity changed; task has not been sent".into());
        }
        // Herdr 0.8.2's startup check accepts fallback idle before Codex renders
        // its trust dialog. Positive idle evidence distinguishes the actual
        // input state without sending text or Enter into a startup dialog.
        let detection = h.call(&["agent", "explain", name, "--json"])?;
        if detection["agent"] != "codex" || !detection["visible_idle"].is_boolean() {
            return Err(
                "Herdr did not return Codex readiness evidence; task has not been sent".into(),
            );
        }
        if detection["state"] == "idle"
            && detection["visible_idle"] == true
            && agent["interactive_ready"] == true
        {
            return Ok(());
        }
        if !reported_blocker
            && (detection["state"] == "blocked" || agent["agent_status"] == "blocked")
        {
            println!("Codex requires input in pane {pane}. Resolve its startup dialog to continue; the task has not been sent.");
            reported_blocker = true;
        }
        if Instant::now() >= deadline {
            return Err(format!("Codex is not ready for task input in pane {pane}; task has not been sent. Inspect its startup dialog").into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn run(state: &Path, id: &str) -> Result<()> {
    let lock = storage::lock(&path(state, id)?.with_extension("lock"))?;
    let mut r = load(state, id)?;
    if r.step != "queued" || r.error.is_some() {
        return Err(format!(
            "Session {id} is {}; no launch replay. {}",
            r.step,
            r.error.as_deref().unwrap_or("")
        )
        .into());
    }
    println!("Preparing session {id}");
    let result = (|| -> Result<()> {
        name_branch(state, &mut r)?;
        if let Some(req) = &r.request {
            for diagnostic in &req.diagnostics {
                println!("{diagnostic}");
            }
        }
        prepare(state, &mut r)?;
        let req = r.request.as_ref().unwrap().clone();
        let receipt = r.receipt.as_ref().unwrap();
        let pane = receipt
            .pane
            .clone()
            .ok_or("prepared workspace has no pane")?;
        if req.focus {
            if req.launch_mode == LaunchMode::Tab {
                r.herdr
                    .call(&["tab", "focus", receipt.tab.as_deref().ok_or("missing tab")?])?;
            } else {
                r.herdr.call(&[
                    "workspace",
                    "focus",
                    receipt.workspace.as_deref().ok_or("missing workspace")?,
                ])?;
            }
        }
        let name = format!("c{}", &r.id[..r.id.len().min(25)]);
        let mut args = vec![
            "agent",
            "start",
            &name,
            "--kind",
            &req.kind,
            "--pane",
            &pane,
            "--timeout",
            "300000",
        ];
        if !req.native_args.is_empty() {
            args.push("--");
            args.extend(req.native_args.iter().map(String::as_str));
        }
        r.step = "starting_agent".into();
        save(state, &r)?;
        let deadline = Instant::now() + Duration::from_secs(300);
        let output = r.herdr.output(&args)?;
        let startup_blocked = req.kind == "codex"
            && !output.success
            && serde_json::from_str::<Value>(&output.stderr)
                .is_ok_and(|v| v["error"]["code"] == "agent_not_ready");
        let started = if startup_blocked {
            // The named process is still alive. Wait for the user to resolve
            // its dialog, without restarting it or submitting the task.
            r.herdr.call(&["agent", "get", &name])?
        } else {
            output.json()?
        };
        if !startup_blocked
            && started.pointer("/result/type").and_then(Value::as_str) != Some("agent_started")
        {
            return Err("Herdr did not confirm agent startup".into());
        }
        r.agent = Some(started["result"]["agent"].clone());
        if started["result"]["agent"]["pane_id"] != pane
            || started["result"]["agent"]["agent"] != req.kind
        {
            return Err("agent startup returned a different live identity".into());
        }
        r.step = "agent_started".into();
        save(state, &r)?;
        if req.kind == "codex" {
            println!("Waiting for Codex input readiness in pane {pane}; resolve any startup dialogs there.");
            wait_for_codex_input(&r.herdr, &name, &pane, deadline)?;
        }
        let mut prompt = req.task.clone();
        if !req.attachments.is_empty() {
            prompt.push_str("\n\nAttached images (retained originals):\n");
            for a in &req.attachments {
                prompt.push_str(&serde_json::to_string(&a.path)?);
                prompt.push('\n');
            }
        }
        r.step = "delivery_attempted".into();
        r.delivery = Delivery::Unknown;
        save(state, &r)?;
        let output = r.herdr.output(&[
            "agent",
            "prompt",
            &pane,
            &prompt,
            "--wait",
            "--until",
            "working",
            "--until",
            "done",
            "--until",
            "idle",
            "--until",
            "blocked",
            "--timeout",
            "10000",
        ])?;
        let response: Value = serde_json::from_str(if output.success {
            &output.stdout
        } else {
            &output.stderr
        })
        .unwrap_or(json!({"stdout":output.stdout,"stderr":output.stderr}));
        r.prompt_result = Some(response.clone());
        if output.success
            && response.pointer("/result/type").and_then(Value::as_str) == Some("agent_prompted")
        {
            r.delivery = Delivery::Confirmed;
            r.step = "delivered".into();
            save(state, &r)?;
            if let Some((p, revision)) = &r.draft {
                storage::clear_draft(p, *revision)?;
            }
            println!("Delivered {id}. Requested agent={} model={:?} effort={:?} speed={:?}. Workspace {}",req.agent,req.model,req.effort,req.speed,r.receipt.as_ref().unwrap().workspace.as_deref().unwrap_or("unknown"));
            Ok(())
        } else {
            if response.pointer("/error/code").and_then(Value::as_str) == Some("agent_blocked") {
                r.delivery = Delivery::NotSent;
            }
            save(state, &r)?;
            Err(format!(
                "prompt {:?}: {response}. Inspect the agent before sending anything again",
                r.delivery
            )
            .into())
        }
    })();
    if let Err(e) = result {
        r.error = Some(e.to_string());
        save(state, &r)?;
        return Err(format!(
            "Session {id} needs attention: {e}. Workspace {:?}; inspect {}. No automatic retry.",
            r.receipt.as_ref().and_then(|p| p.workspace.as_ref()),
            path(state, id)?.display()
        )
        .into());
    }
    // Closing the runner's pane can terminate this process before the command
    // returns. Delivery and draft cleanup must be durable, and the record lock
    // released, before asking Herdr to close it.
    drop(lock);
    if let Some(pane) = &r.runner_pane {
        r.herdr.call(&["pane", "close", pane]).map_err(|e| {
            format!("Session {id} delivered, but preparation pane {pane} could not close: {e}")
        })?;
    }
    Ok(())
}
pub fn current(
    state: &Path,
    checkout: Option<&Path>,
    workspace: Option<&str>,
    tab: Option<&str>,
    h: &Herdr,
) -> Result<String> {
    let mut matches = vec![];
    let mut exact_tabs = vec![];
    if let Ok(entries) = fs::read_dir(state.join("sessions")) {
        for entry in entries {
            let p = entry?.path();
            if p.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let r: SessionRecord = storage::read_json(&p)?;
            if r.version != VERSION {
                return Err("unsupported session version".into());
            }
            if r.step == "removed" || r.herdr.socket != h.socket {
                continue;
            }
            if let Some(receipt) = r.receipt {
                let checkout_matches = checkout
                    .is_some_and(|p| fs::canonicalize(p).ok().as_ref() == Some(&receipt.checkout));
                let workspace_matches =
                    workspace.is_some_and(|w| receipt.workspace.as_deref() == Some(w));
                if (checkout.is_none() || checkout_matches)
                    && (workspace.is_none() || workspace_matches)
                    && (checkout_matches || workspace_matches)
                {
                    if r.request
                        .as_ref()
                        .is_some_and(|r| r.launch_mode == LaunchMode::Tab)
                    {
                        if tab.is_some() && tab == receipt.tab.as_deref() {
                            exact_tabs.push(r.id);
                        }
                    } else {
                        matches.push(r.id);
                    }
                }
            }
        }
    }
    if !exact_tabs.is_empty() {
        matches = exact_tabs;
    }
    if matches.len() != 1 {
        return Err(format!(
            "--current requires one recorded session in the caller's context; matches: {matches:?}"
        )
        .into());
    }
    Ok(matches.remove(0))
}
pub fn removal_summary(state: &Path, id: &str) -> Result<String> {
    let r = load(state, id)?;
    let req = r.request.as_ref().ok_or("session already removed")?;
    let receipt = r
        .receipt
        .as_ref()
        .ok_or("no validated provider receipt; inspect resources manually")?;
    let agents = r.herdr.call(&["agent", "list"])?;
    let live: Vec<_> = agents
        .pointer("/result/agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| {
            a["workspace_id"].as_str() == receipt.workspace.as_deref()
                && (req.launch_mode != LaunchMode::Tab
                    || a["tab_id"].as_str() == receipt.tab.as_deref())
        })
        .cloned()
        .collect();
    let dirty =
        process::git(&receipt.checkout, &["status", "--short"]).unwrap_or_else(|e| e.to_string());
    Ok(format!("Session {id}\nLaunch in {}\nProvider {}\nCheckout {}\nBranch {}\nWorkspace {:?}\nTab {:?}\nLive agents: {}\nPending work:\n{}\n{}",req.launch_mode.label(),req.provider.id,receipt.checkout.display(),receipt.branch,receipt.workspace,receipt.tab,serde_json::to_string(&live)?,dirty,if req.launch_mode==LaunchMode::Tab {"Cleanup closes only the recorded tab. Checkout and files are kept.\n"} else {""}))
}
pub fn remove_from_caller(paths: &Paths, id: &str) -> Result<()> {
    let mut r = load(&paths.state, id)?;
    let receipt = r.receipt.as_ref().ok_or("session has no cleanup receipt")?;
    let in_target = if r
        .request
        .as_ref()
        .is_some_and(|r| r.launch_mode == LaunchMode::Tab)
    {
        env::var_os("COMPOSER_REMOVE_SESSION").is_some()
            || (env::var("HERDR_SOCKET_PATH").ok().as_deref() == Some(r.herdr.socket.as_str())
                && env::var("HERDR_TAB_ID").ok().as_deref() == receipt.tab.as_deref())
    } else {
        env::current_dir()
            .ok()
            .is_some_and(|p| p.starts_with(&receipt.checkout))
            || (env::var("HERDR_SOCKET_PATH").ok().as_deref() == Some(r.herdr.socket.as_str())
                && env::var("HERDR_WORKSPACE_ID").ok().as_deref() == receipt.workspace.as_deref())
            || env::var_os("COMPOSER_REMOVE_SESSION").is_some()
    };
    if !in_target {
        return remove(&paths.state, id);
    }
    let guard = storage::lock(&path(&paths.state, id)?.with_extension("lock"))?;
    r = load(&paths.state, id)?;
    if let Some(pane) = &r.cleanup_pane {
        return Err(format!(
            "Cleanup is already queued in pane {pane}; inspect that pane and the session record"
        )
        .into());
    }
    let source = r
        .source_workspace
        .as_deref()
        .ok_or("missing source workspace; run remove --session from an external terminal")?;
    let tab = r.herdr.call(&[
        "tab",
        "create",
        "--workspace",
        source,
        "--cwd",
        "/",
        "--label",
        "Composer removing",
        "--no-focus",
    ])?;
    let pane = field(&tab, "/result/root_pane/pane_id")?;
    r.cleanup_pane = Some(pane.clone());
    save(&paths.state, &r)?;
    drop(guard);
    let command = format!(
        "HERDR_PLUGIN_STATE_DIR={} COMPOSER_STATE_DIR={} {} __remove {}",
        process::quote(&paths.state.to_string_lossy()),
        process::quote(&paths.state.to_string_lossy()),
        process::quote(&env::current_exe()?.to_string_lossy()),
        process::quote(id)
    );
    r.herdr
        .output(&["pane", "run", &pane, &command])?
        .checked()?;
    println!(
        "Removing {id} from source pane {pane}. Inspect {} for the outcome.",
        path(&paths.state, id)?.display()
    );
    Ok(())
}
pub fn remove(state: &Path, id: &str) -> Result<()> {
    let _lock = storage::lock(&path(state, id)?.with_extension("lock"))?;
    let mut r = load(state, id)?;
    if r.step == "removed" {
        return Err("session already removed; no cleanup replay".into());
    }
    let req = r.request.clone().ok_or("missing request")?;
    let receipt = r
        .receipt
        .clone()
        .ok_or("no validated receipt; inspect partial resources manually")?;
    if req.provider.version != VERSION || receipt.version != VERSION {
        return Err("unsupported cleanup protocol; restore a compatible Composer version".into());
    }
    let result = (|| -> Result<()> {
        if r.step == "removing" {
            return Err(
                "previous removal outcome is uncertain; inspect the provider before retrying"
                    .into(),
            );
        }
        if r.step != "provider_removed" {
            validate_receipt(&req, &receipt)?;
            validate_binding(&r)?;
            r.step = "removing".into();
            r.error = None;
            save(state, &r)?;
            let outcome = if req.launch_mode == LaunchMode::Tab {
                let output = r.herdr.output(&[
                    "tab",
                    "close",
                    receipt.tab.as_deref().ok_or("missing tab ownership")?,
                ])?;
                if !output.success {
                    r.step = "removal_failed".into();
                    save(state, &r)?;
                }
                json!({"output":output.checked()?,"tab":receipt.tab,"checkout_kept":true})
            } else {
                match req.provider.id.as_str() {
                    "herdr" => {
                        let output = r.herdr.output(&[
                            "worktree",
                            "remove",
                            "--workspace",
                            receipt
                                .workspace
                                .as_deref()
                                .ok_or("native receipt has no workspace")?,
                        ])?;
                        if !output.success {
                            r.step = "removal_failed".into();
                            r.removal =
                                Some(json!({"stdout":output.stdout,"stderr":output.stderr}));
                            save(state, &r)?;
                        }
                        let text = output.checked()?;
                        if text.trim().is_empty() {
                            Value::Null
                        } else {
                            serde_json::from_str(&text)?
                        }
                    }
                    "worktrunk" => {
                        let output = process::run(
                            &[
                                "wt".into(),
                                "remove".into(),
                                receipt.checkout.to_string_lossy().into_owned(),
                            ],
                            &req.repository,
                            None,
                            Duration::from_secs(300),
                        )?;
                        if !output.success {
                            r.step = "removal_failed".into();
                            r.removal =
                                Some(json!({"stdout":output.stdout,"stderr":output.stderr}));
                            save(state, &r)?;
                        }
                        let report = output.checked()?;
                        json!({"output":report})
                    }
                    _ => {
                        if !req
                            .provider
                            .command
                            .first()
                            .is_some_and(|p| process::available(p))
                        {
                            return Err("recorded provider is unavailable; restore its original executable to remove this session".into());
                        }
                        let v=process::run(&req.provider.command,&req.repository,Some(&json!({"version":VERSION,"operation":"remove","launch_id":id,"receipt":receipt,"cleanup":req.provider.cleanup})),Duration::from_secs(300))?.json()?;
                        if v["version"] != VERSION || v["status"] != "removed" {
                            return Err(format!("provider removal incomplete: {v}").into());
                        }
                        v
                    }
                }
            };
            let kept = process::git(
                &req.repository,
                &[
                    "show-ref",
                    "--verify",
                    &format!("refs/heads/{}", receipt.branch),
                ],
            )
            .is_ok();
            r.removal = Some(
                json!({"launch_mode":req.launch_mode,"provider":req.provider.id,"outcome":outcome,"branch":receipt.branch,"branch_kept":kept}),
            );
            r.step = "provider_removed".into();
            save(state, &r)?;
        }
        if req.launch_mode == LaunchMode::Worktree && req.provider.id != "herdr" {
            if let Some(ws) = &receipt.workspace {
                validate_binding(&r)?;
                r.herdr.call(&["workspace", "close", ws])?;
            }
        }
        println!("Removed {id}: {}", r.removal.as_ref().unwrap());
        // Retain only a replay guard and cleanup result. Task text and agent
        // transcripts do not remain in successfully removed records.
        r.request = None;
        r.receipt = None;
        r.agent = None;
        r.prompt_result = None;
        r.draft = None;
        r.error = None;
        r.step = "removed".into();
        save(state, &r)
    })();
    if let Err(e) = result {
        r.error = Some(e.to_string());
        save(state, &r)?;
        return Err(format!("Cleanup needs attention for {id}: {e}").into());
    }
    Ok(())
}
