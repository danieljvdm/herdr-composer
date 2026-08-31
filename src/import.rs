use crate::{
    config::{Config, Paths},
    request::Draft,
    storage, Result,
};
use std::{fs, path::Path};

pub fn run(paths: &Paths, source: &Path, preview: bool) -> Result<()> {
    let source = fs::canonicalize(source)?;
    for target in [&paths.state, &paths.config] {
        if fs::canonicalize(target).ok().as_ref() == Some(&source) {
            return Err("import source and destination must differ".into());
        }
    }
    let config_path = if source.join("config.toml").is_file() {
        source.join("config.toml")
    } else {
        source.join("config/config.toml")
    };
    let state = if source.join("state").is_dir() {
        source.join("state")
    } else {
        source.clone()
    };
    let mut config = Config::default();
    config.defaults.workspace = "worktrunk".into();
    println!(
        "Import {} into {} and {}",
        source.display(),
        paths.config.display(),
        paths.state.display()
    );
    if config_path.exists() {
        let old: toml::Table = toml::from_str(&fs::read_to_string(&config_path)?)?;
        for (key, value) in old {
            match key.as_str() {
                "default_agent" => {
                    config.defaults.agent = value
                        .as_str()
                        .ok_or("legacy default_agent must be a string")?
                        .into();
                    println!("default_agent -> defaults.agent");
                }
                "dispatch_focus" => {
                    config.defaults.focus = value
                        .as_bool()
                        .ok_or("legacy dispatch_focus must be boolean")?;
                    println!("dispatch_focus -> defaults.focus");
                }
                "disabled_agents" => {
                    let text = value
                        .as_str()
                        .ok_or("legacy disabled_agents must be a string")?;
                    for name in text
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|s| !s.is_empty())
                    {
                        config.agents.entry(name.into()).or_default().enabled = false;
                    }
                    println!("disabled_agents -> agents.<id>.enabled = false");
                }
                _ => println!("Ignored legacy key: {key}"),
            }
        }
    }
    let destination = paths.config.join("config.toml");
    if destination.exists() {
        println!("Keep existing {}", destination.display());
    } else if !preview {
        storage::write_new(&destination, toml::to_string_pretty(&config)?.as_bytes())?;
    }
    if let Ok(entries) = fs::read_dir(state.join("drafts")) {
        let mut entries = entries.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
        for entry in entries {
            let path = entry.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let mut value: serde_json::Value = storage::read_json(&path)?;
            if value.get("version").is_some_and(|v| v != crate::VERSION) {
                return Err(
                    format!("unsupported legacy draft version in {}", path.display()).into(),
                );
            }
            value["version"] = serde_json::json!(crate::VERSION);
            value["revision"] = serde_json::json!(0);
            value["provider"] = serde_json::json!("worktrunk");
            value["launch_mode"] = serde_json::json!("worktree");
            let mut draft: Draft = serde_json::from_value(value)?;
            let canonical = crate::request::checkout(Path::new(&draft.repo)).ok();
            let context = if draft.repo.is_empty() {
                None
            } else {
                Some(
                    canonical
                        .as_deref()
                        .unwrap_or_else(|| Path::new(&draft.repo)),
                )
            };
            let destination = storage::draft_path(&paths.state, context);
            println!("Draft {} -> {}", path.display(), destination.display());
            if preview {
                if destination.exists() {
                    println!("Keep existing destination draft; retain imported copy in imports/");
                }
                continue;
            }
            for attachment in &mut draft.attachments {
                let original = Path::new(&attachment.path);
                let original = if original.is_file() {
                    original.to_path_buf()
                } else {
                    state
                        .join("attachments")
                        .join(original.file_name().ok_or("attachment has no filename")?)
                };
                let (mut retained, _) =
                    crate::images::import_file(&original, &paths.state.join("attachments"))?;
                retained.name = attachment.name.clone();
                *attachment = retained;
            }
            use sha2::{Digest, Sha256};
            let archive = paths
                .state
                .join("imports/worktrunk")
                .join(format!(
                    "{:x}",
                    Sha256::digest(source.to_string_lossy().as_bytes())
                ))
                .join(path.file_name().ok_or("missing draft filename")?);
            let _archive_lock = storage::lock(&archive.with_extension("lock"))?;
            if !archive.exists() {
                storage::write_json(&archive, &draft)?;
            }
            let _lock = storage::lock(&destination.with_extension("lock"))?;
            if !destination.exists() {
                draft.revision = 1;
                storage::write_json(&destination, &draft)?;
            } else {
                println!(
                    "Keep existing destination draft; imported copy: {}",
                    archive.display()
                );
            }
        }
    }
    println!("Task shortcut: worktrunk.dispatch -> composer.compose. Keep worktrunk.remove-current for general worktree cleanup; composer.remove-current handles recorded Composer sessions only.\nOptional bin/sow wrapper forwards task launches to Composer. Keep reap on Worktrunk.\nSource files, installed commands, checkouts, and cleanup receipts were not modified.");
    Ok(())
}
