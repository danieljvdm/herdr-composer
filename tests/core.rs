use herdr_composer::{
    catalog::{self, Catalog},
    config::Config,
    request::{self, Draft},
    storage,
};
use std::{fs, path::PathBuf};

#[test]
fn launch_mode_defaults_and_saved_choices_are_typed() {
    use herdr_composer::request::LaunchMode;
    assert_eq!(Config::default().defaults.launch_mode, LaunchMode::Worktree);
    let c: Config = toml::from_str("[defaults]\nlaunch_mode='tab'").unwrap();
    assert_eq!(c.defaults.launch_mode, LaunchMode::Tab);
    assert!(toml::from_str::<Config>("[defaults]\nlaunch_mode='elsewhere'").is_err());
    let old: Draft = serde_json::from_str("{\"task\":\"old draft\"}").unwrap();
    assert_eq!(old.launch_mode, None);
    let selected = Draft {
        launch_mode: Some(LaunchMode::Tab),
        ..old
    };
    let restored: Draft = serde_json::from_str(&serde_json::to_string(&selected).unwrap()).unwrap();
    assert_eq!(selected, restored);
}
#[test]
fn branch_naming_is_optional_and_separate_from_task_defaults() {
    let c: Config = toml::from_str("[branch_naming]\nenabled=true\nmodel='fixture-namer'\neffort='medium'\nspeed='fast'\nprefix='team/'").unwrap();
    assert!(c.branch_naming.enabled);
    assert_eq!(c.branch_naming.model, "fixture-namer");
    assert!(c.defaults.model.is_empty());
    assert!(!Config::default().branch_naming.enabled);
}
fn temp() -> PathBuf {
    let p = std::env::temp_dir().join(format!("composer-test-{}", request::launch_id()));
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn literal_directives_stop_at_first_prose_word() {
    let task =
        "@codex >repo branch:fix literal `quotes`\n$(touch nope)\n日本語 >not-a-directive  \n";
    let (d, text) = request::directives(task);
    assert_eq!(d.agent, "codex");
    assert_eq!(d.repo, "repo");
    assert_eq!(d.branch, "fix");
    assert_eq!(
        text,
        "literal `quotes`\n$(touch nope)\n日本語 >not-a-directive  \n"
    );
    assert_eq!(
        request::directives("  regular prose\n").1,
        "  regular prose\n"
    );
}
#[test]
fn catalog_overrides_are_exact_and_preserve_omitted_capabilities() {
    let config: Config = toml::from_str(
        r#"
        [agents.claude]
        catalog = "curated"
        [[agents.claude.models]]
        id = "sonnet"
        label = "Daily"
        aliases = ["daily"]
        visible = false
        efforts = ["low", "high"]
        default_effort = "high"
    "#,
    )
    .unwrap();
    let cat = Catalog::load(&config, true).unwrap();
    let (_, _, m) = cat.selection("claude", "daily", "").unwrap();
    let m = m.unwrap();
    assert_eq!(m.label, "Daily");
    assert!(!m.visible());
    assert_eq!(m.efforts, vec!["low", "high"]);
    assert_eq!(cat.selection("", "daily", "").unwrap().0, "claude");
    let mut c = config.clone();
    c.agents.get_mut("claude").unwrap().enabled = false;
    assert!(Catalog::load(&c, true)
        .unwrap()
        .selection("claude", "daily", "")
        .is_err());
}
#[test]
fn duplicate_catalog_names_fail_and_unknown_models_gain_no_capabilities() {
    let c: Config = toml::from_str(
        r#"
        [agents.codex]
        allow_custom_model = true
        [[agents.codex.models]]
        id = "one"
        aliases = ["two"]
        [[agents.codex.models]]
        id = "two"
    "#,
    )
    .unwrap();
    assert!(Catalog::load(&c, true).is_err());
    let c: Config = toml::from_str("[agents.codex]\nallow_custom_model=true").unwrap();
    let cat = Catalog::load(&c, true).unwrap();
    let m = cat.selection("codex", "new-model", "").unwrap().2.unwrap();
    assert!(m.efforts.is_empty());
    assert!(m.speeds.is_empty());
    assert!(cat.selection("", "new-model", "codex").is_err());
}
#[test]
fn normal_speed_is_an_explicit_native_override() {
    assert_eq!(
        catalog::native_args("codex", None, None, Some("normal")).unwrap(),
        vec!["-c", "service_tier=\"default\""]
    );
    assert!(catalog::native_args("codex", None, None, None)
        .unwrap()
        .is_empty());
    assert!(catalog::native_args("opencode", Some("model"), None, None).is_err());
    assert!(catalog::native_args("opencode", None, None, None)
        .unwrap()
        .is_empty());
}
#[test]
fn stale_editors_cannot_overwrite_or_clear_newer_drafts() {
    let root = temp();
    let path = storage::draft_path(&root, None);
    let mut first = Draft {
        task: "first".into(),
        ..Draft::default()
    };
    storage::save_draft(&path, &mut first).unwrap();
    let mut stale = first.clone();
    first.task = "newer".into();
    storage::save_draft(&path, &mut first).unwrap();
    stale.task = "stale".into();
    assert!(storage::save_draft(&path, &mut stale).is_err());
    storage::clear_draft(&path, 1).unwrap();
    assert_eq!(storage::load_draft(&path).unwrap().unwrap().task, "newer");
    storage::clear_draft(&path, 2).unwrap();
    let cleared = storage::load_draft(&path).unwrap().unwrap();
    assert!(cleared.task.is_empty());
    assert_eq!(cleared.revision, 3);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn persisted_versions_and_exclusive_launch_ownership_are_checked() {
    let root = temp();
    let path = storage::draft_path(&root, None);
    let d = Draft {
        version: 999,
        ..Draft::default()
    };
    storage::write_json(&path, &d).unwrap();
    assert!(storage::load_draft(&path).is_err());
    let guard = storage::lock(&root.join("launch.lock")).unwrap();
    assert!(storage::lock(&root.join("launch.lock")).is_err());
    drop(guard);
    assert!(storage::lock(&root.join("launch.lock")).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn subprocess_timeout_and_output_limits_are_bounded() {
    use herdr_composer::process;
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let error = process::run(
        &["sh".into(), "-c".into(), "sleep 2".into()],
        std::path::Path::new("/"),
        None,
        Duration::from_millis(40),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(1));
    let error = process::run(
        &["sh".into(), "-c".into(), "yes x".into()],
        std::path::Path::new("/"),
        None,
        Duration::from_secs(2),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("exceeded"));
}

#[test]
fn current_base_is_pinned_to_invoking_checkout_and_cannot_cross_repositories() {
    use herdr_composer::process::git;
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]).unwrap();
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    )
    .unwrap();
    let linked = root.join("linked");
    git(
        &repo,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    )
    .unwrap();
    git(
        &linked,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "topic",
        ],
    )
    .unwrap();
    let head = git(&linked, &["rev-parse", "HEAD"]).unwrap();
    // Validation reaches base checks before requiring an installed agent.
    let d = Draft {
        task: "task".into(),
        repo: repo.to_string_lossy().into_owned(),
        repo_explicit: true,
        base: "current".into(),
        ..Draft::default()
    };
    let c = Config::default();
    let cat = Catalog::load(&c, false).unwrap();
    assert!(request::resolve(&d, &c, &cat, None, &root)
        .unwrap_err()
        .to_string()
        .contains("Current checkout"));
    if herdr_composer::process::available("codex") {
        let mut c = c;
        c.defaults.agent = "codex".into();
        let r = request::resolve(&d, &c, &cat, Some(&linked), &root).unwrap();
        assert_eq!(r.base_commit.as_deref(), Some(head.as_str()));
        assert_eq!(r.repository, fs::canonicalize(&repo).unwrap());
        assert_eq!(
            r.invoking_checkout,
            Some(fs::canonicalize(&linked).unwrap())
        );
    }
    fs::remove_dir_all(root).unwrap();
}
