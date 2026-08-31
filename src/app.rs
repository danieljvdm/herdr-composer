use crate::images::{self, Attachment, Preview};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_launch_mode: herdr_composer::request::LaunchMode,
    pub repo: String,
    pub repos: Vec<String>,
    pub agents: Vec<String>,
    pub catalog: herdr_composer::catalog::Catalog,
    pub providers: Vec<String>,
    pub default_provider: String,
    pub current_repositories: Vec<String>,
    pub default_agent: String,
    pub branch: String,
    pub base: String,
    pub focus: bool,
    pub message: String,
    pub task: String,
    pub attachment_dir: String,
    pub attachments: Vec<Attachment>,
}

pub use herdr_composer::request::Draft;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    LaunchMode,
    Task,
    Images,
    Repo,
    Agent,
    Provider,
    Base,
    Follow,
    More,
    Branch,
    Model,
    Effort,
    Speed,
    Launch,
    Close,
}
#[derive(PartialEq)]
pub enum Outcome {
    Continue,
    Close,
    Submit,
}

pub struct Picker {
    pub field: Field,
    pub query: TextArea<'static>,
    pub options: Vec<(String, String)>,
    pub selected: usize,
    pub free_text: bool,
    pub filtering: bool,
}
impl Picker {
    fn down(&mut self) {
        self.selected = (self.selected + 1).min(self.matches().len().saturating_sub(1));
    }
    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn matches(&self) -> Vec<(String, String)> {
        let query = self.query.lines().join("").to_lowercase();
        self.options
            .iter()
            .filter(|(value, label)| format!("{label} {value}").to_lowercase().contains(&query))
            .cloned()
            .collect()
    }
}

pub struct App {
    pub config: Config,
    pub settings: Draft,
    pub text: TextArea<'static>,
    pub focused: Field,
    last_setting: Field,
    pub more: bool,
    pub picker: Option<Picker>,
    pub hits: Vec<(Rect, Field)>,
    pub picker_hits: Vec<(Rect, usize)>,
    pub image_hits: Vec<(Rect, usize)>,
    pub previews: Vec<Preview>,
    pub image_index: usize,
    pub image_view: bool,
    pub graphics: Option<crate::graphics::Graphics>,
    pub image_placements: Vec<(usize, Rect)>,
    pub message: String,
    pub saved: bool,
    pub editor_rect: Rect,
    pub editor_cursor: Option<Position>,
}

pub fn editor(value: &str) -> TextArea<'static> {
    let mut area = TextArea::from(value.split('\n').map(String::from).collect::<Vec<_>>());
    area.set_wrap_mode(WrapMode::WordOrGlyph);
    area.move_cursor(CursorMove::Bottom);
    area.move_cursor(CursorMove::End);
    area
}

impl App {
    pub fn launch_mode(&self) -> herdr_composer::request::LaunchMode {
        self.settings
            .launch_mode
            .unwrap_or(self.config.default_launch_mode)
    }
    pub fn new(config: Config, draft: Option<Draft>) -> Self {
        let restored = draft.is_some();
        let settings = draft.unwrap_or_else(|| Draft {
            task: config.task.clone(),
            attachments: config.attachments.clone(),
            repo: config.repo.clone(),
            base: config.base.clone(),
            branch: config.branch.clone(),
            focus: None,
            ..Draft::default()
        });
        let previews = settings
            .attachments
            .iter()
            .map(|a| Preview::load(Path::new(&a.path)))
            .collect();
        let mut text = editor(&settings.task);
        text.set_placeholder_text("Describe the task. Paste text or an image to add context.");
        let more = !settings.branch.is_empty()
            || !settings.model.is_empty()
            || !settings.base.is_empty()
            || !settings.speed.is_empty();
        Self {
            message: config.message.clone(),
            config,
            settings,
            text,
            focused: Field::Task,
            last_setting: Field::Repo,
            more,
            picker: None,
            hits: vec![],
            picker_hits: vec![],
            image_hits: vec![],
            previews,
            image_index: 0,
            image_view: false,
            graphics: None,
            image_placements: vec![],
            saved: restored,
            editor_rect: Rect::default(),
            editor_cursor: None,
        }
    }
    pub fn draft(&self) -> Draft {
        Draft {
            task: self.text.lines().join("\n"),
            ..self.settings.clone()
        }
    }
    pub fn supported(&self, field: Field) -> Vec<String> {
        self.config
            .catalog
            .selection(
                &self.settings.agent,
                &self.settings.model,
                &self.config.default_agent,
            )
            .ok()
            .and_then(|(_, _, m)| m)
            .map(|m| {
                if field == Field::Effort {
                    m.efforts
                } else {
                    m.speeds
                }
            })
            .unwrap_or_default()
    }
    pub fn fields(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::Task,
            Field::Repo,
            Field::LaunchMode,
            Field::Agent,
            Field::Model,
        ];
        if self.launch_mode() == herdr_composer::request::LaunchMode::Worktree {
            fields.insert(3, Field::Provider);
        }
        if !self.settings.attachments.is_empty() {
            fields.insert(1, Field::Images);
        }
        if !self.supported(Field::Effort).is_empty() || !self.settings.effort.is_empty() {
            fields.push(Field::Effort);
        }
        if !self.supported(Field::Speed).is_empty() || !self.settings.speed.is_empty() {
            fields.push(Field::Speed);
        }
        fields.extend([Field::Follow, Field::More]);
        if self.more
            && (self.launch_mode() == herdr_composer::request::LaunchMode::Worktree
                || !self.settings.branch.is_empty()
                || !self.settings.base.is_empty())
        {
            fields.extend([Field::Branch, Field::Base]);
        }
        fields.extend([Field::Launch, Field::Close]);
        fields
    }
    fn tab(&mut self, backwards: bool) {
        let fields = self.fields();
        let index = fields.iter().position(|f| *f == self.focused).unwrap_or(0);
        self.focused =
            fields[(index + if backwards { fields.len() - 1 } else { 1 }) % fields.len()];
    }
    pub fn value(&self, field: Field) -> String {
        match field {
            Field::LaunchMode => match self.settings.launch_mode {
                Some(mode) => mode.label().into(),
                None => format!("Auto · {}", self.config.default_launch_mode.label()),
            },
            Field::Repo => {
                if self.settings.repo.is_empty() {
                    "Choose repository".into()
                } else {
                    short_repo(&self.settings.repo)
                }
            }
            Field::Provider => or_default(
                &self.settings.provider,
                &format!("Auto · {}", self.config.default_provider),
            ),
            Field::Agent => {
                if self.settings.agent.is_empty() {
                    format!("Auto · {}", self.config.default_agent)
                } else {
                    self.settings.agent.clone()
                }
            }
            Field::Base => match self.settings.base.as_str() {
                "" => "Provider default".into(),
                "@" => "Current checkout".into(),
                v => v.into(),
            },
            Field::Branch => or_default(&self.settings.branch, "Generated task name"),
            Field::Model => {
                let fallback = self
                    .config
                    .catalog
                    .agents
                    .get(if self.settings.agent.is_empty() {
                        &self.config.default_agent
                    } else {
                        &self.settings.agent
                    })
                    .map(|a| a.default_model.as_str())
                    .unwrap_or("");
                let token = if self.settings.model.is_empty() {
                    fallback
                } else {
                    &self.settings.model
                };
                let label = self
                    .config
                    .catalog
                    .selection(&self.settings.agent, token, &self.config.default_agent)
                    .ok()
                    .and_then(|(_, _, m)| m)
                    .map(|m| m.label)
                    .unwrap_or_else(|| token.into());
                if self.settings.model.is_empty() {
                    if label.is_empty() {
                        "Automatic".into()
                    } else {
                        format!("Auto · {label}")
                    }
                } else {
                    label
                }
            }
            Field::Effort | Field::Speed => {
                let value = if field == Field::Effort {
                    &self.settings.effort
                } else {
                    &self.settings.speed
                };
                let fallback = self
                    .config
                    .catalog
                    .selection(
                        &self.settings.agent,
                        &self.settings.model,
                        &self.config.default_agent,
                    )
                    .ok()
                    .and_then(|(_, _, m)| m)
                    .map(|m| {
                        if field == Field::Effort {
                            m.default_effort
                        } else {
                            m.default_speed
                        }
                    })
                    .unwrap_or_default();
                if value.is_empty() && !fallback.is_empty() {
                    format!("Auto · {fallback}")
                } else {
                    or_default(value, "Automatic")
                }
            }
            Field::Follow => if self.settings.focus.unwrap_or(self.config.focus) {
                "[x] Open workspace"
            } else {
                "[ ] Open workspace"
            }
            .into(),
            Field::More => if self.more {
                "▾ Fewer options"
            } else {
                "▸ More options"
            }
            .into(),
            _ => String::new(),
        }
    }
    pub fn launch_label(&self) -> String {
        if self.settings.agent.is_empty() {
            "Launch task ▸".into()
        } else {
            format!("Launch {} ▸", self.settings.agent)
        }
    }
    fn open(&mut self, field: Field) -> Outcome {
        self.focused = field;
        let (options, initial, free_text) = match field {
            Field::LaunchMode => (
                vec![
                    (
                        String::new(),
                        format!("Automatic · {}", self.config.default_launch_mode.label()),
                    ),
                    ("worktree".into(), "New worktree".into()),
                    ("tab".into(), "Tab in selected checkout".into()),
                ],
                String::new(),
                false,
            ),
            Field::Repo => (
                self.config
                    .repos
                    .iter()
                    .map(|p| (p.clone(), short_repo(p)))
                    .collect(),
                String::new(),
                true,
            ),
            Field::Agent => {
                let mut choices = vec![(
                    String::new(),
                    format!("Auto · default {}", self.config.default_agent),
                )];
                choices.extend(self.config.agents.iter().map(|id| {
                    (
                        id.clone(),
                        self.config
                            .catalog
                            .agents
                            .get(id)
                            .map(|a| a.label.clone())
                            .unwrap_or_else(|| id.clone()),
                    )
                }));
                (choices, String::new(), false)
            }
            Field::Base => {
                let mut choices = vec![(String::new(), "Provider default".into())];
                if self
                    .config
                    .current_repositories
                    .contains(&self.settings.repo)
                {
                    choices.push(("@".into(), "Current checkout".into()));
                }
                (choices, String::new(), true)
            }
            Field::Provider => {
                let mut choices = vec![(
                    String::new(),
                    format!("Automatic · {}", self.config.default_provider),
                )];
                choices.extend(self.config.providers.iter().map(|p| (p.clone(), p.clone())));
                (choices, String::new(), false)
            }
            Field::Branch => (vec![], self.settings.branch.clone(), true),
            Field::Model => {
                let mut choices = vec![(String::new(), "Automatic".into())];
                let agent = self
                    .config
                    .catalog
                    .agents
                    .get(if self.settings.agent.is_empty() {
                        &self.config.default_agent
                    } else {
                        &self.settings.agent
                    });
                if let Some(a) = agent {
                    choices.extend(
                        a.models
                            .iter()
                            .filter(|m| m.enabled() && m.visible())
                            .map(|m| (m.id.clone(), m.label.clone())),
                    );
                }
                (
                    choices,
                    String::new(),
                    agent.is_some_and(|a| a.allow_custom_model),
                )
            }
            Field::Effort | Field::Speed => {
                let mut choices = vec![(String::new(), "Automatic".into())];
                choices.extend(self.supported(field).into_iter().map(|s| (s.clone(), s)));
                (choices, String::new(), false)
            }
            Field::Follow => {
                self.settings.focus = Some(!self.settings.focus.unwrap_or(self.config.focus));
                self.saved = false;
                return Outcome::Continue;
            }
            Field::More => {
                self.more = !self.more;
                return Outcome::Continue;
            }
            Field::Launch => return self.submit(),
            Field::Close => return Outcome::Close,
            Field::Images => {
                self.image_view = !self.settings.attachments.is_empty();
                return Outcome::Continue;
            }
            Field::Task => return Outcome::Continue,
        };
        let current = match field {
            Field::LaunchMode => match self.settings.launch_mode {
                Some(herdr_composer::request::LaunchMode::Worktree) => "worktree",
                Some(herdr_composer::request::LaunchMode::Tab) => "tab",
                None => "",
            },
            Field::Repo => &self.settings.repo,
            Field::Provider => &self.settings.provider,
            Field::Agent => &self.settings.agent,
            Field::Base => &self.settings.base,
            Field::Effort => &self.settings.effort,
            Field::Speed => &self.settings.speed,
            _ => "",
        };
        let selected = options
            .iter()
            .position(|(value, _)| value == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            field,
            query: editor(&initial),
            options,
            selected,
            free_text,
            filtering: matches!(field, Field::Branch),
        });
        Outcome::Continue
    }
    fn apply_picker(&mut self) {
        let Some(picker) = &self.picker else {
            return;
        };
        let matches = picker.matches();
        let value = if let Some((value, _)) = matches.get(picker.selected) {
            value.clone()
        } else if picker.free_text {
            picker.query.lines().join("").trim().to_string()
        } else {
            return;
        };
        match picker.field {
            Field::LaunchMode => {
                self.settings.launch_mode = if value.is_empty() {
                    None
                } else {
                    herdr_composer::request::LaunchMode::parse(&value).ok()
                }
            }
            Field::Repo => {
                self.settings.repo = value;
                self.settings.repo_explicit = true;
            }
            Field::Provider => self.settings.provider = value,
            Field::Agent => self.settings.agent = value,
            Field::Base => self.settings.base = value,
            Field::Branch => self.settings.branch = value,
            Field::Model => self.settings.model = value,
            Field::Effort => self.settings.effort = value,
            Field::Speed => self.settings.speed = value,
            _ => {}
        }
        self.picker = None;
        self.saved = false;
        self.message.clear();
        if self.launch_mode() == herdr_composer::request::LaunchMode::Tab
            && (!self.settings.branch.is_empty() || !self.settings.base.is_empty())
        {
            self.more = true;
            self.message =
                "Tab mode uses the existing checkout. Clear branch/base in More options to launch."
                    .into();
        }
        for (field, value) in [
            (Field::Effort, &self.settings.effort),
            (Field::Speed, &self.settings.speed),
        ] {
            if !value.is_empty() && !self.supported(field).contains(value) {
                self.message = format!(
                    "Saved {} '{value}' is incompatible. Choose a supported value or Automatic.",
                    if field == Field::Effort {
                        "effort"
                    } else {
                        "speed"
                    }
                );
            }
        }
    }
    fn submit(&mut self) -> Outcome {
        if self.text.lines().join("\n").trim().is_empty() && self.settings.attachments.is_empty() {
            self.message = "Describe the task before launching.".into();
            self.focused = Field::Task;
            return Outcome::Continue;
        }
        if let Some(attachment) = self
            .settings
            .attachments
            .iter()
            .find(|a| !Path::new(&a.path).is_file())
        {
            self.message = format!(
                "Image is missing: {}. Remove it or paste it again.",
                attachment.name
            );
            self.focused = Field::Images;
            return Outcome::Continue;
        }
        Outcome::Submit
    }
    pub fn event(&mut self, event: Event) -> Outcome {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.key(key),
            Event::Paste(value) => {
                if let Some(picker) = &mut self.picker {
                    picker.query.insert_str(value.replace(['\n', '\r'], " "));
                    picker.selected = 0;
                    picker.filtering = true;
                } else if matches!(self.focused, Field::Task | Field::Images) {
                    if value.is_empty() {
                        self.paste_clipboard();
                    } else if let Some(paths) = images::pasted_paths(&value) {
                        for path in paths {
                            let result =
                                images::import_file(&path, Path::new(&self.config.attachment_dir));
                            self.add_image(result);
                        }
                    } else if self.focused == Field::Task {
                        self.text
                            .insert_str(value.replace("\r\n", "\n").replace('\r', "\n"));
                        self.saved = false;
                    }
                }
            }
            Event::Mouse(mouse) => {
                let point = Position::new(mouse.column, mouse.row);
                if self.image_view {
                    return Outcome::Continue;
                }
                if self.picker.is_some() {
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        if let Some((_, index)) = self
                            .picker_hits
                            .iter()
                            .find(|(rect, _)| rect.contains(point))
                        {
                            self.picker.as_mut().unwrap().selected = *index;
                            self.apply_picker();
                        }
                    } else if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) {
                        return self.key(KeyEvent::new(
                            if mouse.kind == MouseEventKind::ScrollDown {
                                KeyCode::Down
                            } else {
                                KeyCode::Up
                            },
                            KeyModifiers::NONE,
                        ));
                    }
                } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    if let Some((_, index)) = self
                        .image_hits
                        .iter()
                        .find(|(rect, _)| rect.contains(point))
                    {
                        self.image_index = *index;
                        return self.open(Field::Images);
                    }
                    if let Some((_, field)) = self
                        .hits
                        .iter()
                        .find(|(rect, _)| rect.contains(point))
                        .copied()
                    {
                        if field == Field::Task {
                            self.place_cursor(point, false);
                        }
                        return self.open(field);
                    }
                } else if mouse.kind == MouseEventKind::Drag(MouseButton::Left)
                    && self.editor_rect.contains(point)
                {
                    self.place_cursor(point, true);
                } else if self.editor_rect.contains(point)
                    && matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    )
                {
                    self.text.input(mouse);
                } else if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                ) {
                    self.tab(mouse.kind == MouseEventKind::ScrollUp);
                }
            }
            _ => {}
        }
        Outcome::Continue
    }
    // Use the widget's wrapped movement and the rendered cursor to locate
    // clicks without maintaining a second text layout implementation.
    fn place_cursor(&mut self, point: Position, selecting: bool) {
        let Some(cursor) = self.editor_cursor else {
            self.focused = Field::Task;
            return;
        };
        if selecting {
            if self.text.selection_range().is_none() {
                self.text.start_selection();
            }
        } else {
            self.text.cancel_selection();
        }
        let rows = i32::from(point.y) - i32::from(cursor.y);
        for _ in 0..rows.unsigned_abs() {
            self.text.move_cursor(if rows < 0 {
                CursorMove::Up
            } else {
                CursorMove::Down
            });
        }
        let target_col = usize::from(point.x.saturating_sub(self.editor_rect.x));
        let target_row = self.text.screen_cursor().row;
        for _ in 0..self.editor_rect.width {
            let before = self.text.screen_cursor();
            if before.col == target_col {
                break;
            }
            let backwards = before.col > target_col;
            self.text.move_cursor(if backwards {
                CursorMove::Back
            } else {
                CursorMove::Forward
            });
            let after = self.text.screen_cursor();
            if after.row != target_row {
                self.text.move_cursor(if backwards {
                    CursorMove::Forward
                } else {
                    CursorMove::Back
                });
                break;
            }
            if before.col == after.col
                || (backwards && after.col <= target_col)
                || (!backwards && after.col >= target_col)
            {
                break;
            }
        }
        self.focused = Field::Task;
    }
    fn add_image(&mut self, result: Result<(Attachment, Preview), String>) {
        match result {
            Ok((attachment, preview)) => {
                if let Some(index) = self
                    .settings
                    .attachments
                    .iter()
                    .position(|a| a.path == attachment.path)
                {
                    self.image_index = index;
                    self.previews[index] = preview;
                } else {
                    self.image_index = self.settings.attachments.len();
                    self.settings.attachments.push(attachment);
                    self.previews.push(preview);
                }
                self.saved = false;
                self.message.clear();
            }
            Err(error) => self.message = error,
        }
    }
    fn paste_clipboard(&mut self) {
        let result = images::clipboard_image().and_then(|bytes| {
            images::import_bytes(
                &bytes,
                "clipboard.png",
                Path::new(&self.config.attachment_dir),
            )
        });
        self.add_image(result);
    }
    fn remove_image(&mut self) {
        if self.settings.attachments.is_empty() {
            return;
        }
        self.settings.attachments.remove(self.image_index);
        self.previews.remove(self.image_index);
        self.image_index = self
            .image_index
            .min(self.settings.attachments.len().saturating_sub(1));
        if self.settings.attachments.is_empty() {
            self.image_view = false;
            self.focused = Field::Task;
        }
        self.saved = false;
        self.message.clear();
    }
    fn image_step(&mut self, backwards: bool) {
        let count = self.settings.attachments.len();
        if count > 0 {
            self.image_index = (self.image_index + if backwards { count - 1 } else { 1 }) % count;
        }
    }
    fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.image_view {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.image_view = false,
                KeyCode::Char('h' | 'c') if key.modifiers == KeyModifiers::CONTROL => {
                    self.image_view = false
                }
                KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k') => self.image_step(true),
                KeyCode::Right | KeyCode::Down | KeyCode::Char('l' | 'j') => self.image_step(false),
                KeyCode::Delete | KeyCode::Backspace => self.remove_image(),
                _ => {}
            }
            return Outcome::Continue;
        }
        if self.picker.is_none()
            && key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('v')
        {
            self.paste_clipboard();
            return Outcome::Continue;
        }

        // Handle focus chords before the editor can consume Ctrl+H/J/K as
        // editing commands. Enter and the actual Backspace key still edit.
        if key.modifiers == KeyModifiers::CONTROL {
            if let KeyCode::Char(direction @ ('h' | 'j' | 'k' | 'l')) = key.code {
                if let Some(picker) = &mut self.picker {
                    match direction {
                        'h' => self.picker = None,
                        'j' => picker.down(),
                        'k' => picker.up(),
                        'l' => self.apply_picker(),
                        _ => unreachable!(),
                    }
                } else {
                    match direction {
                        'h' => {
                            if !matches!(
                                self.focused,
                                Field::Task | Field::Images | Field::Launch | Field::Close
                            ) {
                                self.last_setting = self.focused;
                            }
                            self.focused = Field::Task;
                        }
                        'l' if matches!(self.focused, Field::Task | Field::Images) => {
                            self.focused = if self.fields().contains(&self.last_setting) {
                                self.last_setting
                            } else {
                                Field::Repo
                            };
                        }
                        'j' => self.tab(false),
                        'k' => self.tab(true),
                        _ => {}
                    }
                }
                return Outcome::Continue;
            }
        }
        if let Some(picker) = &mut self.picker {
            match key.code {
                KeyCode::Esc if picker.filtering && !matches!(picker.field, Field::Branch) => {
                    picker.filtering = false;
                }
                KeyCode::Esc => self.picker = None,
                KeyCode::Enter => self.apply_picker(),
                KeyCode::Down => picker.down(),
                KeyCode::Up => picker.up(),
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    picker.down()
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => picker.up(),
                KeyCode::Char('j') if !picker.filtering && key.modifiers.is_empty() => {
                    picker.down()
                }
                KeyCode::Char('k') if !picker.filtering && key.modifiers.is_empty() => picker.up(),
                KeyCode::Char('g') if !picker.filtering => picker.selected = 0,
                KeyCode::Char('G') if !picker.filtering => {
                    picker.selected = picker.matches().len().saturating_sub(1)
                }
                KeyCode::Char('/') if !picker.filtering => picker.filtering = true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.picker = None
                }
                _ => {
                    picker.filtering = true;
                    picker.query.input(key);
                    picker.selected = 0;
                }
            }
            return Outcome::Continue;
        }
        if key.code == KeyCode::Esc {
            return Outcome::Close;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => return self.submit(),
                KeyCode::Char('c')
                    if self.focused != Field::Task || self.text.selection_range().is_none() =>
                {
                    return Outcome::Close
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Tab => self.tab(false),
            KeyCode::BackTab => self.tab(true),
            KeyCode::Left | KeyCode::Char('h') if self.focused == Field::Images => {
                self.image_step(true)
            }
            KeyCode::Right | KeyCode::Char('l') if self.focused == Field::Images => {
                self.image_step(false)
            }
            KeyCode::Delete | KeyCode::Backspace if self.focused == Field::Images => {
                self.remove_image()
            }
            _ if self.focused == Field::Task => {
                if self.text.input(key) {
                    self.saved = false;
                    self.message.clear();
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => return self.open(self.focused),
            KeyCode::Down | KeyCode::Char('j') => self.tab(false),
            KeyCode::Up | KeyCode::Char('k') => self.tab(true),
            _ => {}
        }
        Outcome::Continue
    }
}

pub fn short_repo(value: &str) -> String {
    Path::new(value)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| value.into())
}
fn or_default(value: &str, default: &str) -> String {
    if value.is_empty() {
        default.into()
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    #[test]
    fn multiline_paste_and_undo_preserve_literal_task() {
        let mut app = App::new(Config::default(), None);
        app.event(Event::Paste(
            "Fix `auth`\r\n$(touch /tmp/never)\n日本語 🐑".into(),
        ));
        assert_eq!(
            app.draft().task,
            "Fix `auth`\n$(touch /tmp/never)\n日本語 🐑"
        );
        app.text.undo();
        assert!(app.draft().task.is_empty());
        app.text.redo();
        assert!(app.draft().task.ends_with("日本語 🐑"));
    }
    #[test]
    fn cancel_retains_draft_and_settings() {
        let saved = Draft {
            task: "Do work\nCarefully".into(),
            repo: "/tmp/repo".into(),
            agent: "codex".into(),
            speed: "normal".into(),
            ..Draft::default()
        };
        let mut app = App::new(Config::default(), Some(saved.clone()));
        assert!(app.event(key(KeyCode::Esc)) == Outcome::Close);
        assert_eq!(app.draft(), saved);
    }
    #[test]
    fn picker_cancellation_does_not_change_settings() {
        let mut app = App::new(
            Config {
                agents: vec!["codex".into(), "grok".into()],
                ..Config::default()
            },
            None,
        );
        app.open(Field::Agent);
        app.event(key(KeyCode::Down));
        app.event(key(KeyCode::Esc));
        assert!(app.settings.agent.is_empty());
        app.open(Field::Agent);
        app.event(key(KeyCode::Down));
        app.event(key(KeyCode::Enter));
        assert_eq!(app.settings.agent, "codex");
    }

    #[test]
    fn vim_navigation_and_filter_text_are_separate() {
        let mut app = App::new(
            Config {
                agents: vec!["codex".into(), "grok".into()],
                ..Config::default()
            },
            None,
        );
        app.open(Field::Agent);
        app.event(key(KeyCode::Char('j')));
        assert_eq!(app.picker.as_ref().unwrap().selected, 1);
        assert!(app.picker.as_ref().unwrap().query.is_empty());
        app.event(key(KeyCode::Char('k')));
        assert_eq!(app.picker.as_ref().unwrap().selected, 0);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.picker.as_ref().unwrap().selected, 1);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.picker.as_ref().unwrap().selected, 0);
        app.event(key(KeyCode::Char('/')));
        for c in "grok".chars() {
            app.event(key(KeyCode::Char(c)));
        }
        assert_eq!(app.picker.as_ref().unwrap().query.lines().join(""), "grok");
        app.event(key(KeyCode::Esc));
        assert!(!app.picker.as_ref().unwrap().filtering);
        app.event(key(KeyCode::Enter));
        assert_eq!(app.settings.agent, "grok");
    }

    #[test]
    fn vim_letters_remain_text_in_editors() {
        let mut app = App::new(Config::default(), None);
        for c in "jk/gG".chars() {
            app.event(key(KeyCode::Char(c)));
        }
        assert_eq!(app.draft().task, "jk/gG");
        app.open(Field::Branch);
        for c in "jk/gG".chars() {
            app.event(key(KeyCode::Char(c)));
        }
        app.event(key(KeyCode::Enter));
        assert_eq!(app.settings.branch, "jk/gG");
    }

    #[test]
    fn control_focus_chords_preserve_text_and_restore_last_setting() {
        let mut app = App::new(
            Config {
                task: "Keep this text".into(),
                ..Config::default()
            },
            None,
        );
        let control = |c| Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        app.event(control('l'));
        assert_eq!(app.focused, Field::Repo);
        app.event(control('j'));
        assert_eq!(app.focused, Field::LaunchMode);
        app.event(control('h'));
        assert_eq!(app.focused, Field::Task);
        assert_eq!(app.draft().task, "Keep this text");
        app.event(control('l'));
        assert_eq!(app.focused, Field::LaunchMode);
        app.event(control('k'));
        assert_eq!(app.focused, Field::Repo);
        app.event(control('h'));
        app.event(key(KeyCode::Enter));
        assert_eq!(app.draft().task, "Keep this text\n");
        app.event(key(KeyCode::Backspace));
        assert_eq!(app.draft().task, "Keep this text");
    }
}
