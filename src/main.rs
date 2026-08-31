mod app;
mod editor;
mod graphics;
mod ui;
use herdr_composer::{
    catalog::Catalog,
    config::Paths,
    images,
    request::{self, Draft},
    session, Result,
};
use std::{
    env,
    io::{self, Read},
    path::{Path, PathBuf},
};

fn main() {
    if let Err(e) = run() {
        eprintln!("Herdr Composer: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let paths = Paths::discover();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("Herdr Composer\n\nherdr-composer                         Open editor\nherdr-composer launch [OPTIONS] TEXT   Launch a task, or use - for stdin\nherdr-composer catalog --json          Inspect resolved agent/model catalog\nherdr-composer remove --session ID     Remove a recorded session\nherdr-composer remove --current        Pin and remove the caller's session\nherdr-composer import-worktrunk PATH [--preview]\n\nLaunch options: --launch-mode worktree|tab --repo PATH|NAME --provider ID --branch NAME --base REF|current\n                --agent ID --model ID|ALIAS --effort VALUE --speed VALUE\n                --focus --no-focus --attach PATH (repeatable)\nUse -- before task text that begins with a dash.\nEditor: Ctrl+S launch, Ctrl+R refresh catalog, Esc save and close.");
            Ok(())
        }
        Some("--version") => {
            println!("herdr-composer {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("__run") => session::run(&paths.state, args.get(1).ok_or("missing session ID")?),
        Some("__remove") => session::remove(&paths.state, args.get(1).ok_or("missing session ID")?),
        Some("__action") => action(&paths, args.get(1).ok_or("missing action")?),
        Some("__confirm-remove") => {
            let id = env::var("COMPOSER_REMOVE_SESSION")?;
            println!(
                "{}Remove this session? [y/N]",
                session::removal_summary(&paths.state, &id)?
            );
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if answer.trim().eq_ignore_ascii_case("y") {
                session::remove_from_caller(&paths, &id)?;
            }
            Ok(())
        }
        Some("remove") => {
            let id = match args.get(1).map(String::as_str) {
                Some("--session") if args.len() == 3 => args[2].clone(),
                Some("--current") if args.len() == 2 => {
                    let h = session::Herdr::current()?;
                    let cwd = request::checkout(&env::current_dir()?).ok();
                    session::current(
                        &paths.state,
                        cwd.as_deref(),
                        env::var("HERDR_WORKSPACE_ID").ok().as_deref(),
                        env::var("HERDR_TAB_ID").ok().as_deref(),
                        &h,
                    )?
                }
                _ => return Err("usage: remove --session ID | --current".into()),
            };
            println!("{}", session::removal_summary(&paths.state, &id)?);
            session::remove_from_caller(&paths, &id)
        }
        Some("import-worktrunk") => herdr_composer::import::run(
            &paths,
            Path::new(args.get(1).ok_or("missing old state path")?),
            args.iter().any(|s| s == "--preview"),
        ),
        Some("catalog") => {
            if args.len() != 2 || args[1] != "--json" {
                return Err("usage: catalog --json".into());
            }
            let c = paths.load()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&Catalog::load(&c, true)?)?
            );
            Ok(())
        }
        Some("launch") => {
            let d = parse_launch(&args[1..])?;
            let mut c = paths.load()?;
            c.add_open_repositories();
            let cat = Catalog::load(&c, true)?;
            let invoking = request::checkout(&env::current_dir()?).ok();
            let req = request::resolve(&d, &c, &cat, invoking.as_deref(), &paths.state)?;
            for diagnostic in &req.diagnostics {
                eprintln!("{diagnostic}");
            }
            let id = session::submit(&paths, req, None)?;
            println!(
                "Preparing session {id}. Record: {}",
                session::path(&paths.state, &id)?.display()
            );
            Ok(())
        }
        Some("--snapshot") => editor::snapshot(&args[1..]),
        None => editor::run(&paths),
        _ => Err("unknown command; see herdr-composer --help".into()),
    }
}
fn parse_launch(args: &[String]) -> Result<Draft> {
    let mut d = Draft::default();
    let mut i = 0;
    let mut text = None;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--focus" => d.focus = Some(true),
            "--no-focus" => d.focus = Some(false),
            "--repo" | "--provider" | "--launch-mode" | "--branch" | "--base" | "--agent"
            | "--model" | "--effort" | "--speed" | "--attach" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("{arg} needs a value"))?
                    .clone();
                if v.is_empty() {
                    return Err(format!("{arg} cannot be empty; omit it for Automatic").into());
                }
                match arg.as_str() {
                    "--repo" => {
                        d.repo = v;
                        d.repo_explicit = true
                    }
                    "--provider" => d.provider = v,
                    "--launch-mode" => d.launch_mode = Some(request::LaunchMode::parse(&v)?),
                    "--branch" => d.branch = v,
                    "--base" => d.base = v,
                    "--agent" => d.agent = v,
                    "--model" => d.model = v,
                    "--effort" => d.effort = v,
                    "--speed" => d.speed = v,
                    _ => d.attachments.push(images::Attachment {
                        path: v,
                        name: String::new(),
                    }),
                }
            }
            "--" => {
                i += 1;
                if args.len() != i + 1 {
                    return Err("supply one quoted task argument".into());
                }
                text = Some(args[i].clone());
                break;
            }
            "-" => {
                if text.is_some() {
                    return Err("task supplied more than once".into());
                }
                let mut s = String::new();
                io::stdin().take(1024 * 1024 + 1).read_to_string(&mut s)?;
                if s.len() > 1024 * 1024 {
                    return Err("task exceeds 1 MiB".into());
                }
                text = Some(s)
            }
            _ if arg.starts_with('-') => return Err(format!("unknown launch option: {arg}").into()),
            _ => {
                if text.replace(arg.clone()).is_some() {
                    return Err("supply one quoted task argument".into());
                }
            }
        }
        i += 1;
    }
    d.task = text.unwrap_or_default();
    Ok(d)
}
fn action(paths: &Paths, action: &str) -> Result<()> {
    let context: serde_json::Value = serde_json::from_str(&env::var("HERDR_PLUGIN_CONTEXT_JSON")?)?;
    let cwd = context["workspace_cwd"]
        .as_str()
        .or_else(|| context["focused_pane_cwd"].as_str());
    let h = session::Herdr::current()?;
    let mut args = vec![
        "plugin".to_string(),
        "pane".into(),
        "open".into(),
        "--plugin".into(),
        "composer".into(),
        "--entrypoint".into(),
    ];
    let pinned = if action == "remove-current" {
        let checkout = context
            .pointer("/worktree/checkout_path")
            .and_then(|v| v.as_str())
            .or(cwd)
            .map(PathBuf::from);
        let id = session::current(
            &paths.state,
            checkout.as_deref(),
            context["workspace_id"].as_str(),
            context["tab_id"].as_str(),
            &h,
        )?;
        args.push("remove-current".into());
        Some(format!("COMPOSER_REMOVE_SESSION={id}"))
    } else if action == "compose" {
        args.push("composer".into());
        None
    } else {
        return Err("unknown plugin action".into());
    };
    args.extend([
        "--placement".into(),
        "overlay".into(),
        "--cwd".into(),
        env::var("HERDR_PLUGIN_ROOT")?,
        "--focus".into(),
    ]);
    if let Some(pinned) = pinned {
        args.extend(["--env".into(), pinned]);
    }
    // Herdr 0.8.2's PTY launcher resolves argv[0] through PATH, independently
    // of the pane cwd. Scope the package binary directory to this pane.
    let mut executable_paths = vec![PathBuf::from(env::var("HERDR_PLUGIN_ROOT")?).join("bin")];
    executable_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    args.extend([
        "--env".into(),
        format!(
            "PATH={}",
            env::join_paths(executable_paths)?.to_string_lossy()
        ),
    ]);
    if action == "compose" {
        args.extend([
            "--env".into(),
            format!("COMPOSER_INVOKING_CHECKOUT={}", cwd.unwrap_or("")),
        ]);
    }
    h.call(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    Ok(())
}
