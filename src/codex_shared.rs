use crate::{
    config::SharedCodex,
    process,
    request::TaskRequest,
    session::{Herdr, Receipt},
    Result,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

const BRIDGE: &str = include_str!("codex_shared.py");

pub fn validate(shared: &SharedCodex) -> Result<()> {
    if !shared
        .socket
        .strip_prefix("unix://")
        .is_some_and(|path| Path::new(path).is_absolute())
    {
        return Err("codex.shared.socket must be unix:///absolute/path/to/server.sock".into());
    }
    if !shared.contexts_dir.is_absolute() {
        return Err("codex.shared.contexts_dir must be absolute".into());
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct Prepared {
    pub thread_id: Option<String>,
    pub error: Option<String>,
    #[serde(skip)]
    pub routing_args: Vec<String>,
}

pub fn prepare(
    req: &TaskRequest,
    receipt: &Receipt,
    herdr: &Herdr,
    shared: &SharedCodex,
) -> Result<Prepared> {
    validate(shared)?;
    let workspace = receipt.workspace.as_deref().ok_or("missing workspace")?;
    let pane = receipt.pane.as_deref().ok_or("missing pane")?;
    let panes = herdr.call(&["pane", "list", "--workspace", workspace])?;
    let live = panes["result"]["panes"]
        .as_array()
        .and_then(|panes| panes.iter().find(|live| live["pane_id"] == pane))
        .ok_or("shared Codex destination pane is no longer present")?;
    let tab = live["tab_id"].as_str().ok_or("missing destination tab")?;
    let input = json!({
        "endpoint": shared.socket, "contexts_dir": shared.contexts_dir,
        "launch_id": req.launch_id, "name": format!("Composer {}", req.branch),
        "cwd": receipt.checkout, "model": req.model, "effort": req.effort, "speed": req.speed,
        "environment": {
            "HERDR_ENV": "1", "HERDR_SOCKET_PATH": herdr.socket,
            "HERDR_BIN_PATH": herdr.binary, "HERDR_WORKSPACE_ID": workspace,
            "HERDR_PANE_ID": pane, "HERDR_TAB_ID": tab, "HERDR_STARTUP_CWD": receipt.checkout,
        },
    });
    let argv = vec![
        "python3".into(),
        "-c".into(),
        BRIDGE.into(),
        "prepare".into(),
    ];
    let output = process::run(
        &argv,
        &receipt.checkout,
        Some(&input),
        Duration::from_secs(45),
    )?;
    let mut result: Prepared = serde_json::from_str(&output.stdout)
        .map_err(|_| format!("shared Codex preparation failed: {}", output.stderr))?;
    if !output.success && result.error.is_none() {
        return Err("shared Codex preparation failed without a diagnostic".into());
    }
    if result.error.is_none() && result.thread_id.is_none() {
        return Err("shared Codex did not return a session ID".into());
    }
    for (key, value) in input["environment"].as_object().unwrap() {
        result.routing_args.extend([
            "-c".into(),
            format!("shell_environment_policy.set.{key}={value}"),
        ]);
    }
    Ok(result)
}

pub fn hook(args: &[String]) -> Result<()> {
    if args.len() < 5 || args[1] != "--contexts" || args[3] != "--" {
        return Err("usage: __codex-hook --contexts DIRECTORY -- COMMAND".into());
    }
    let mode = if args[0] == "__codex-notify" {
        "notify"
    } else {
        "hook"
    };
    let status = Command::new("python3")
        .args(["-c", BRIDGE, mode, &args[2]])
        .args(&args[4..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_backend_requires_explicit_local_paths() {
        for socket in [
            "unix://",
            "unix://relative.sock",
            "ws://localhost:1234",
            "https://example.com",
        ] {
            assert!(validate(&SharedCodex {
                socket: socket.into(),
                contexts_dir: "/tmp/contexts".into()
            })
            .is_err());
        }
        assert!(validate(&SharedCodex {
            socket: "unix:///tmp/codex.sock".into(),
            contexts_dir: "relative".into()
        })
        .is_err());
        assert!(validate(&SharedCodex {
            socket: "unix:///tmp/codex.sock".into(),
            contexts_dir: "/tmp/contexts".into()
        })
        .is_ok());
        let old: crate::config::Config = toml::from_str("[defaults]\nagent='codex'\n").unwrap();
        assert!(old.codex.shared.is_none());
    }
}
