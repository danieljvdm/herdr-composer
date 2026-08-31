use crate::{
    app::{App, Config as EditorConfig, Outcome},
    graphics, ui,
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyModifiers,
    },
    execute,
};
use herdr_composer::{
    catalog::Catalog,
    config::{Config, Paths},
    request, session, storage, Result,
};
use ratatui::{backend::TestBackend, Terminal};
use std::{
    env, fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};

fn editor_config(c: &Config, cat: Catalog, paths: &Paths, repo: String) -> EditorConfig {
    let mut agents: Vec<_> = cat
        .agents
        .iter()
        .filter(|(_, a)| a.enabled && a.visible)
        .collect();
    agents.sort_by_key(|(id, a)| (a.order, *id));
    let agents = agents.into_iter().map(|(id, _)| id.clone()).collect();
    let mut repos = c.repositories.clone();
    if !repo.is_empty() && !repos.contains(&repo) {
        repos.push(repo.clone());
    }
    let common = request::common(std::path::Path::new(&repo)).ok();
    let current_repositories = if common.is_some() {
        repos
            .iter()
            .filter(|p| request::common(std::path::Path::new(p)).ok() == common)
            .cloned()
            .collect()
    } else {
        vec![]
    };
    EditorConfig {
        repo,
        repos,
        agents,
        catalog: cat,
        providers: std::iter::once("herdr".into())
            .chain(std::iter::once("worktrunk".into()))
            .chain(c.providers.keys().cloned())
            .collect(),
        default_provider: c.defaults.workspace.clone(),
        default_launch_mode: c.defaults.launch_mode,
        current_repositories,
        default_agent: c.defaults.agent.clone(),
        focus: c.defaults.focus,
        attachment_dir: paths
            .state
            .join("attachments")
            .to_string_lossy()
            .into_owned(),
        ..EditorConfig::default()
    }
}
fn discover(c: Config) -> std::sync::mpsc::Receiver<Result<Catalog>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Catalog::load(&c, true));
    });
    rx
}
pub fn run(paths: &Paths) -> Result<()> {
    let mut config = paths.load()?;
    config.add_open_repositories();
    let invoking = match env::var("COMPOSER_INVOKING_CHECKOUT") {
        Ok(s) if s.is_empty() => None,
        Ok(s) => request::checkout(std::path::Path::new(&s)).ok(),
        Err(_) => request::checkout(&env::current_dir()?).ok(),
    };
    let repo = invoking
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let draft_path = storage::draft_path(&paths.state, invoking.as_deref());
    let draft = storage::load_draft(&draft_path)?;
    let mut app = App::new(
        editor_config(&config, Catalog::load(&config, false)?, paths, repo.clone()),
        draft,
    );
    let mut discovery = Some(discover(config.clone()));
    app.message = "Loading catalog… Ctrl+R refreshes".into();
    app.graphics = graphics::Graphics::from_env();
    let mut terminal = ratatui::init();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        ratatui::restore();
        previous(info);
    }));
    let result = (|| -> Result<()> {
        execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
        let mut last_saved = app.draft();
        let mut last_edit = Instant::now();
        loop {
            if let Some(rx) = &discovery {
                if let Ok(result) = rx.try_recv() {
                    match result {
                        Ok(cat) => {
                            app.message = cat.diagnostics.join("; ");
                            app.config = editor_config(&config, cat, paths, repo.clone());
                        }
                        Err(e) => app.message = format!("Catalog: {e}"),
                    };
                    discovery = None;
                }
            }
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if let Some(graphics) = &mut app.graphics {
                graphics.sync(
                    &app.image_placements,
                    &mut app.previews,
                    &app.settings.attachments,
                );
            }
            if event::poll(Duration::from_millis(100))? {
                let event = event::read()?;
                if matches!(&event,Event::Key(k)if k.code==KeyCode::Char('r')&&k.modifiers.contains(KeyModifiers::CONTROL))
                {
                    if discovery.is_none() {
                        match paths.load() {
                            Ok(mut c) => {
                                c.add_open_repositories();
                                config = c;
                                discovery = Some(discover(config.clone()));
                                app.message = "Loading catalog…".into();
                            }
                            Err(e) => app.message = e.to_string(),
                        }
                    }
                    continue;
                }
                if matches!(event, Event::Key(_) | Event::Paste(_)) {
                    last_edit = Instant::now();
                }
                let outcome = app.event(event);
                if outcome == Outcome::Submit || outcome == Outcome::Close {
                    let mut draft = app.draft();
                    if draft != last_saved || !draft_path.exists() {
                        if let Err(e) = storage::save_draft(&draft_path, &mut draft) {
                            if outcome == Outcome::Close
                                && storage::load_draft(&draft_path)?
                                    .is_some_and(|d| d.revision != app.settings.revision)
                            {
                                let recovery = paths
                                    .state
                                    .join("draft-conflicts")
                                    .join(format!("{}.json", request::launch_id()));
                                storage::write_json(&recovery, &app.draft())?;
                                return Err(format!("A newer draft was preserved. This editor's task was saved to {}",recovery.display()).into());
                            }
                            app.message = e.to_string();
                            continue;
                        }
                        app.settings.revision = draft.revision;
                        last_saved = draft.clone();
                    }
                    if outcome == Outcome::Close {
                        break;
                    }
                    if discovery.is_some() {
                        app.message="Catalog is still loading. Your task is saved; launch again when it finishes.".into();
                        continue;
                    }
                    app.message = "Preparing task…".into();
                    terminal.draw(|f| ui::draw(f, &mut app))?;
                    let result = request::resolve(
                        &draft,
                        &config,
                        &app.config.catalog,
                        invoking.as_deref(),
                        &paths.state,
                    )
                    .and_then(|req| {
                        session::submit(paths, req, Some((draft_path.clone(), draft.revision)))
                    });
                    match result {
                        Ok(id) => {
                            app.message = format!("Preparing session {id}");
                            terminal.draw(|f| ui::draw(f, &mut app))?;
                            break;
                        }
                        Err(e) => {
                            app.message = e.to_string();
                        }
                    }
                }
            }
            let mut draft = app.draft();
            if draft != last_saved && last_edit.elapsed() >= Duration::from_millis(350) {
                match storage::save_draft(&draft_path, &mut draft) {
                    Ok(()) => {
                        app.settings.revision = draft.revision;
                        last_saved = draft;
                        app.saved = true;
                    }
                    Err(e) => {
                        app.message = e.to_string();
                        last_edit = Instant::now();
                    }
                }
            }
        }
        Ok(())
    })();
    drop(app.graphics.take());
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result
}
pub fn snapshot(args: &[String]) -> Result<()> {
    let config: EditorConfig = serde_json::from_slice(&fs::read(PathBuf::from(
        args.first().ok_or("missing snapshot input")?,
    ))?)?;
    let mut app = App::new(config, None);
    let cols = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(110);
    let rows = args.get(2).map(|v| v.parse()).transpose()?.unwrap_or(32);
    let mut terminal = Terminal::new(TestBackend::new(cols, rows))?;
    terminal.draw(|f| ui::draw(f, &mut app))?;
    for y in 0..rows {
        println!(
            "{}",
            (0..cols)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        );
    }
    Ok(())
}
