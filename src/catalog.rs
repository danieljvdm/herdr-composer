use crate::{config::Config, process, Result, VERSION};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    time::Duration,
};

fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(from = "ModelWire")]
pub struct Model {
    pub id: String,
    pub label: String,
    pub aliases: Vec<String>,
    pub order: i32,
    pub visible: Option<bool>,
    pub enabled: Option<bool>,
    pub efforts: Vec<String>,
    pub speeds: Vec<String>,
    pub default_effort: String,
    pub default_speed: String,
    #[serde(skip)]
    pub specified: Vec<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ModelWire {
    id: String,
    label: Option<String>,
    aliases: Option<Vec<String>>,
    order: Option<i32>,
    visible: Option<bool>,
    enabled: Option<bool>,
    efforts: Option<Vec<String>>,
    speeds: Option<Vec<String>>,
    default_effort: Option<String>,
    default_speed: Option<String>,
}
impl From<ModelWire> for Model {
    fn from(w: ModelWire) -> Self {
        let mut specified = vec!["id".into()];
        macro_rules! field {
            ($name:ident) => {{
                if w.$name.is_some() {
                    specified.push(stringify!($name).into());
                }
                w.$name.unwrap_or_default()
            }};
        }
        let label = field!(label);
        let aliases = field!(aliases);
        let order = field!(order);
        let efforts = field!(efforts);
        let speeds = field!(speeds);
        let default_effort = field!(default_effort);
        let default_speed = field!(default_speed);
        if w.visible.is_some() {
            specified.push("visible".into());
        }
        if w.enabled.is_some() {
            specified.push("enabled".into());
        }
        Self {
            id: w.id,
            label,
            aliases,
            order,
            efforts,
            speeds,
            default_effort,
            default_speed,
            visible: w.visible,
            enabled: w.enabled,
            specified,
        }
    }
}
impl Model {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn visible(&self) -> bool {
        self.visible.unwrap_or(true)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub kind: String,
    pub label: String,
    pub order: i32,
    pub enabled: bool,
    pub visible: bool,
    pub catalog: String,
    pub command: Vec<String>,
    pub allow_custom_model: bool,
    pub default_model: String,
    pub models: Vec<Model>,
}
impl Default for Agent {
    fn default() -> Self {
        Self {
            kind: String::new(),
            label: String::new(),
            order: 0,
            enabled: yes(),
            visible: yes(),
            catalog: String::new(),
            command: vec![],
            allow_custom_model: false,
            default_model: String::new(),
            models: vec![],
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Catalog {
    pub version: u32,
    pub agents: BTreeMap<String, Agent>,
    pub diagnostics: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelList {
    version: u32,
    models: Vec<Model>,
}
pub const KINDS: &[&str] = &[
    "pi",
    "claude",
    "codex",
    "gemini",
    "cursor",
    "devin",
    "agy",
    "cline",
    "omp",
    "mastracode",
    "opencode",
    "copilot",
    "kimi",
    "kiro",
    "droid",
    "amp",
    "grok",
    "hermes",
    "kilo",
    "qodercli",
    "qwen",
    "maki",
];

fn validate(models: &[Model], capabilities: bool) -> Result<()> {
    let mut names = HashSet::new();
    for m in models {
        if m.id.is_empty() {
            return Err("empty model ID".into());
        }
        for name in std::iter::once(&m.id).chain(&m.aliases) {
            if name.is_empty() || !names.insert(name) {
                return Err(format!("duplicate model ID or conflicting alias: {name}").into());
            }
        }
        if capabilities && !m.default_effort.is_empty() && !m.efforts.contains(&m.default_effort) {
            return Err(format!("unsupported default effort for {}", m.id).into());
        }
        if capabilities && !m.default_speed.is_empty() && !m.speeds.contains(&m.default_speed) {
            return Err(format!("unsupported default speed for {}", m.id).into());
        }
    }
    Ok(())
}
impl Catalog {
    pub fn load(config: &Config, discover: bool) -> Result<Self> {
        let mut agents = config.agents.clone();
        for kind in KINDS {
            if process::available(kind) {
                agents.entry(kind.to_string()).or_default();
            }
        }
        let mut diagnostics = vec![];
        for (id, a) in &mut agents {
            if a.kind.is_empty() {
                a.kind = id.clone();
            }
            if !KINDS.contains(&a.kind.as_str()) {
                return Err(format!("unsupported Herdr agent kind: {}", a.kind).into());
            }
            if a.label.is_empty() {
                a.label = id.clone();
            }
            if a.catalog.is_empty() {
                a.catalog = if a.kind == "codex" {
                    "discovery"
                } else {
                    "curated"
                }
                .into();
            }
            validate(&a.models, false)?;
            let source = if !discover {
                Ok(vec![])
            } else {
                match a.catalog.as_str() {
                    "curated" => {
                        let all: BTreeMap<String, Vec<Model>> =
                            serde_json::from_str(include_str!("../catalogs/curated.json"))?;
                        Ok(all.get(&a.kind).cloned().unwrap_or_default())
                    }
                    "command" => (|| -> Result<Vec<Model>> {
                        let output = process::run(
                            &a.command,
                            Path::new("/"),
                            Some(&serde_json::json!({"version":VERSION,"agent":id,"kind":a.kind})),
                            Duration::from_secs(5),
                        )?;
                        if !output.stderr.trim().is_empty() {
                            diagnostics.push(format!("{id}: {}", output.stderr.trim()));
                        }
                        let list: ModelList = serde_json::from_str(&output.checked()?)?;
                        if list.version != VERSION {
                            return Err("unsupported catalog version".into());
                        }
                        Ok(list.models)
                    })(),
                    "discovery" if a.kind == "codex" => discover_codex(),
                    "discovery" => Err(format!(
                        "{} has no built-in discovery; configure a catalog command",
                        a.kind
                    )
                    .into()),
                    other => return Err(format!("unknown catalog source: {other}").into()),
                }
            };
            let mut models = match source.and_then(|m| {
                validate(&m, true)?;
                Ok(m)
            }) {
                Ok(m) => m,
                Err(e) => {
                    diagnostics.push(format!("{id}: {e}"));
                    vec![]
                }
            };
            for patch in &a.models {
                if let Some(m) = models.iter_mut().find(|m| m.id == patch.id) {
                    let mut merged = serde_json::to_value(&*m)?;
                    let fields = serde_json::to_value(patch)?;
                    for key in &patch.specified {
                        merged[key] = fields[key].clone();
                    }
                    *m = serde_json::from_value(merged)?;
                } else {
                    models.push(patch.clone());
                }
            }
            validate(&models, discover)?;
            for m in &mut models {
                if m.label.is_empty() {
                    m.label = m.id.clone();
                }
                m.enabled = Some(m.enabled());
                m.visible = Some(m.visible());
            }
            models.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
            a.models = models;
        }
        Ok(Self {
            version: VERSION,
            agents,
            diagnostics,
        })
    }
    pub fn selection(
        &self,
        agent: &str,
        model: &str,
        default_agent: &str,
    ) -> Result<(String, Agent, Option<Model>)> {
        let id = if !agent.is_empty() {
            agent.to_string()
        } else if !model.is_empty() {
            let matches: Vec<_> = self
                .agents
                .iter()
                .filter(|(_, a)| {
                    a.enabled
                        && a.models
                            .iter()
                            .any(|m| m.id == model || m.aliases.iter().any(|s| s == model))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if matches.len() != 1 {
                return Err(format!(
                    "model '{model}' requires an explicit agent; matches: {}",
                    matches.join(", ")
                )
                .into());
            }
            matches[0].clone()
        } else if !default_agent.is_empty() {
            default_agent.into()
        } else {
            let ids: Vec<_> = self
                .agents
                .iter()
                .filter(|(_, a)| a.enabled && process::available(&a.kind))
                .map(|(id, _)| id.clone())
                .collect();
            if ids.len() != 1 {
                return Err(format!("Choose an agent: {}", ids.join(", ")).into());
            }
            ids[0].clone()
        };
        let a = self
            .agents
            .get(&id)
            .ok_or_else(|| format!("unknown agent: {id}"))?;
        if !a.enabled {
            return Err(format!("agent {id} is disabled").into());
        }
        let token = if model.is_empty() {
            &a.default_model
        } else {
            model
        };
        let m = if token.is_empty() {
            None
        } else if let Some(m) = a
            .models
            .iter()
            .find(|m| m.id == token || m.aliases.iter().any(|s| s == token))
        {
            if !m.enabled() {
                return Err(format!("model {} is disabled", m.id).into());
            }
            Some(m.clone())
        } else if !agent.is_empty() && a.allow_custom_model {
            Some(Model {
                id: token.into(),
                label: token.into(),
                ..Model::default()
            })
        } else {
            return Err(format!("unknown model '{token}' for {id}; custom models require an explicit agent and allow_custom_model").into());
        };
        Ok((id, a.clone(), m))
    }
}
fn discover_codex() -> Result<Vec<Model>> {
    use std::io::Read;
    let root = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        });
    let mut bytes = Vec::new();
    std::fs::File::open(root.join("models_cache.json"))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("Codex discovery output exceeds 1 MiB".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let list = value["models"]
        .as_array()
        .ok_or("Codex discovery has no models")?;
    list.iter()
        .map(|v| {
            Ok(Model {
                id: v["slug"].as_str().ok_or("Codex model has no slug")?.into(),
                label: v["display_name"].as_str().unwrap_or_default().into(),
                efforts: v["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["effort"].as_str().map(String::from))
                    .collect(),
                visible: Some(v["visibility"].as_str() != Some("hide")),
                order: v["priority"].as_i64().unwrap_or(0) as i32,
                ..Model::default()
            })
        })
        .collect()
}

pub fn native_args(
    kind: &str,
    model: Option<&str>,
    effort: Option<&str>,
    speed: Option<&str>,
) -> Result<Vec<String>> {
    let mut args = vec![];
    if model.is_none() && effort.is_none() && speed.is_none() {
        return Ok(args);
    }
    match kind {
        "codex" => {
            if let Some(m) = model {
                args.extend(["--model".into(), m.into()]);
            }
            if let Some(e) = effort {
                args.extend([
                    "-c".into(),
                    format!("model_reasoning_effort={}", serde_json::to_string(e)?),
                ]);
            }
            if let Some(s) = speed {
                let tier = match s {
                    "fast" => "fast",
                    "normal" => "default",
                    _ => return Err(format!("Codex speed has no adapter: {s}").into()),
                };
                args.extend([
                    "-c".into(),
                    format!("service_tier={}", serde_json::to_string(tier)?),
                ]);
            }
        }
        "claude" => {
            if let Some(m) = model {
                args.extend(["--model".into(), m.into()]);
            }
            if let Some(e) = effort {
                args.extend(["--effort".into(), e.into()]);
            }
            if speed.is_some() {
                return Err("Claude speed has no supported launch adapter".into());
            }
        }
        _ => {
            return Err(format!(
                "{kind} supports Automatic settings only; no explicit-setting adapter"
            )
            .into())
        }
    }
    Ok(args)
}
