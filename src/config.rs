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
pub struct Config {
    pub defaults: Defaults,
    pub repositories: Vec<String>,
    pub agents: BTreeMap<String, Agent>,
    pub providers: BTreeMap<String, Provider>,
    pub prose_resolver: Vec<String>,
    pub branch_naming: BranchNaming,
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
        for pane in panes
            .pointer("/result/panes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(cwd) = pane["cwd"].as_str() {
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
        match fs::read_to_string(self.config.join("config.toml")) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }
}
