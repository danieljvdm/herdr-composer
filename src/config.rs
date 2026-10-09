use crate::{catalog::Agent, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    pub launch_mode: crate::request::LaunchMode,
    pub workspace: String,
    pub repo: String,
    pub agent: String,
    pub model: String,
    pub effort: String,
    pub speed: String,
    pub focus: bool,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            launch_mode: crate::request::LaunchMode::Worktree,
            workspace: "herdr".into(),
            repo: String::new(),
            agent: String::new(),
            model: String::new(),
            effort: String::new(),
            speed: String::new(),
            focus: true,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Provider {
    pub command: Vec<String>,
    pub cleanup: serde_json::Value,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BranchNaming {
    pub enabled: bool,
    pub model: String,
    pub effort: String,
    pub speed: String,
    pub prefix: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceNaming {
    pub enabled: bool,
    pub model: String,
    pub effort: String,
    pub speed: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedCodex {
    pub socket: String,
    pub contexts_dir: PathBuf,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Codex {
    pub shared: Option<SharedCodex>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub defaults: Defaults,
    pub repositories: Vec<String>,
    pub agents: BTreeMap<String, Agent>,
    pub providers: BTreeMap<String, Provider>,
    pub prose_resolver: Vec<String>,
    pub branch_naming: BranchNaming,
    pub workspace_naming: WorkspaceNaming,
    pub codex: Codex,
}
#[derive(Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
}
impl Config {
    pub fn add_open_repositories(&mut self) {
        let Ok(h) = crate::session::Herdr::current() else {
            return;
        };
        let Ok(panes) = h.call(&["pane", "list"]) else {
            return;
        };
        let mut seen = std::collections::HashSet::new();
        for pane in panes
            .pointer("/result/panes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(cwd) = pane["cwd"].as_str() {
                if !seen.insert(cwd) {
                    continue;
                }
                if let Ok(root) = crate::request::primary(std::path::Path::new(cwd)) {
                    let p = root.to_string_lossy().into_owned();
                    if !self.repositories.contains(&p) {
                        self.repositories.push(p);
                    }
                }
            }
        }
    }
}
impl Paths {
    pub fn discover() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let config = env::var_os("HERDR_PLUGIN_CONFIG_DIR")
            .or_else(|| env::var_os("COMPOSER_CONFIG_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"))
                    .join("herdr/plugins/config/composer")
            });
        let state = env::var_os("HERDR_PLUGIN_STATE_DIR")
            .or_else(|| env::var_os("COMPOSER_STATE_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/state"))
                    .join("herdr/plugins/composer")
            });
        Self { config, state }
    }
    pub fn load(&self) -> Result<Config> {
        let mut config: Config = match fs::read_to_string(self.config.join("config.toml")) {
            Ok(s) => toml::from_str(&s)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e.into()),
        };
        // A socket and its verified adapter state belong to this machine. Keep
        // this opt-in outside a config.toml managed by shared dotfiles.
        match fs::read_to_string(self.state.join("codex-shared.toml")) {
            Ok(s) => {
                if config.codex.shared.is_some() {
                    return Err(
                        "shared Codex is configured in both config.toml and local state".into(),
                    );
                }
                config.codex.shared = Some(toml::from_str(&s)?);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_backend_activation_stays_machine_local() {
        let root = env::temp_dir().join(format!("composer-config-{}", crate::request::launch_id()));
        let paths = Paths {
            config: root.join("config"),
            state: root.join("state"),
        };
        fs::create_dir_all(&paths.config).unwrap();
        fs::create_dir_all(&paths.state).unwrap();
        fs::write(
            paths.config.join("config.toml"),
            "[defaults]\nagent='codex'\n",
        )
        .unwrap();
        assert!(paths.load().unwrap().codex.shared.is_none());
        let shared = "socket='unix:///tmp/server.sock'\ncontexts_dir='/tmp/contexts'\n";
        fs::write(paths.state.join("codex-shared.toml"), shared).unwrap();
        let config = paths.load().unwrap();
        assert_eq!(config.defaults.agent, "codex");
        assert_eq!(
            config.codex.shared.unwrap().socket,
            "unix:///tmp/server.sock"
        );
        fs::write(paths.state.join("codex-shared.toml"), "socket=1").unwrap();
        assert!(paths.load().is_err());
        fs::write(paths.state.join("codex-shared.toml"), shared).unwrap();
        fs::write(
            paths.config.join("config.toml"),
            format!("[codex.shared]\n{shared}"),
        )
        .unwrap();
        assert!(paths.load().is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
