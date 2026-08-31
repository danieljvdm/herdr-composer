use crate::app::{App, Field};
use ratatui::{
    layout::{Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

const BG: Color = Color::Rgb(26, 27, 38);
const PANEL: Color = Color::Rgb(22, 22, 30);
const INK: Color = Color::Rgb(192, 202, 245);
const MUTED: Color = Color::Rgb(137, 148, 186);
const BLUE: Color = Color::Rgb(122, 162, 247);
const LINE: Color = Color::Rgb(52, 59, 88);
const SELECT: Color = Color::Rgb(41, 46, 66);
const RED: Color = Color::Rgb(247, 118, 142);

fn style(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}
fn text(frame: &mut Frame, area: Rect, value: impl Into<String>, fg: Color, bg: Color) {
    frame.render_widget(Paragraph::new(value.into()).style(style(fg, bg)), area);
}
pub fn label(field: Field) -> &'static str {
    match field {
        Field::Repo => "Repository",
        Field::Agent => "Agent",
        Field::Provider => "Workspace provider",
        Field::LaunchMode => "Launch in",
        Field::Base => "Start from",
        Field::Branch => "Branch name",
        Field::Model => "Model",
        Field::Effort => "Reasoning effort",
        Field::Speed => "Speed",
        _ => "",
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.hits.clear();
    app.picker_hits.clear();
    app.image_hits.clear();
    app.image_placements.clear();
    frame.render_widget(Block::default().style(style(INK, BG)), area);
    if area.width < 40 || area.height < 12 {
        text(
            frame,
            area,
            "Enlarge the pane to edit.\nEsc saves your draft and closes.",
            INK,
            BG,
        );
        return;
    }
    let inner = area.inner(Margin::new(2, 1));
    let button_width = (app.launch_label().len() as u16 + 2).min(inner.width.saturating_sub(18));
    let close = Rect::new(inner.right() - 3, inner.y, 3, 1);
    let launch = Rect::new(
        close.x.saturating_sub(button_width + 2),
        inner.y,
        button_width,
        1,
    );
    frame.render_widget(
        Paragraph::new("New task").style(style(INK, BG).add_modifier(Modifier::BOLD)),
        Rect::new(inner.x, inner.y, launch.x.saturating_sub(inner.x), 1),
    );
    let launch_color = if app.draft().task.trim().is_empty() && app.settings.attachments.is_empty()
    {
        MUTED
    } else {
        BLUE
    };
    text(
        frame,
        launch,
        format!(" {} ", app.launch_label()),
        BG,
        if app.focused == Field::Launch {
            INK
        } else {
            launch_color
        },
    );
    text(
        frame,
        close,
        " × ",
        if app.focused == Field::Close {
            BG
        } else {
            MUTED
        },
        if app.focused == Field::Close { INK } else { BG },
    );
    app.hits
        .extend([(launch, Field::Launch), (close, Field::Close)]);
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(style(LINE, BG)),
        Rect::new(area.x, inner.y + 2, area.width, 1),
    );

    let footer_height = if app.message.is_empty() { 2 } else { 4 };
    let body = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(footer_height + 3),
    );
    let wide = area.width >= 86;
    let (writing, settings) = if wide {
        let rail = 29.min(body.width / 3);
        (
            Rect::new(body.x, body.y, body.width - rail - 3, body.height),
            Rect::new(body.right() - rail, body.y, rail, body.height),
        )
    } else {
        let settings_height = if app.more { 10 } else { 6 }.min(body.height.saturating_sub(6));
        (
            Rect::new(
                body.x,
                body.y,
                body.width,
                body.height.saturating_sub(settings_height + 1),
            ),
            Rect::new(
                body.x,
                body.bottom() - settings_height,
                body.width,
                settings_height,
            ),
        )
    };
    text(
        frame,
        Rect::new(writing.x, writing.y, writing.width, 1),
        "What should the agent work on?",
        if app.focused == Field::Task {
            BLUE
        } else {
            MUTED
        },
        BG,
    );
    let image_height = if app.previews.is_empty() {
        0
    } else {
        10.min(writing.height.saturating_sub(5))
    };
    let editor = Rect::new(
        writing.x,
        writing.y + 2,
        writing.width,
        writing.height.saturating_sub(3 + image_height),
    );
    app.editor_rect = editor;
    app.text.set_style(style(INK, BG));
    app.text.set_cursor_line_style(style(INK, BG));
    app.text.set_placeholder_style(style(MUTED, BG));
    app.text.set_selection_style(style(INK, SELECT));
    let cursor_bg = if app.focused == Field::Task && app.picker.is_none() {
        INK
    } else {
        MUTED
    };
    app.text.set_cursor_style(style(BG, cursor_bg));
    frame.render_widget(&app.text, editor);
    app.editor_cursor = None;
    for y in editor.y..editor.bottom() {
        for x in editor.x..editor.right() {
            let cell = &frame.buffer_mut()[(x, y)];
            if cell.bg == cursor_bg && cell.fg == BG {
                app.editor_cursor = Some(Position::new(x, y));
            }
        }
    }
    app.hits.push((editor, Field::Task));
    if image_height > 0 {
        draw_images(
            frame,
            app,
            Rect::new(writing.x, editor.bottom(), writing.width, image_height),
        );
    }
    if writing.height > 3 {
        text(
            frame,
            Rect::new(writing.x, writing.bottom() - 1, writing.width, 1),
            if app.focused == Field::Images {
                "h/l choose · Enter inspect · Delete remove"
            } else {
                "Enter adds a line · Ctrl+V pastes an image"
            },
            MUTED,
            BG,
        );
    }

    if wide {
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(style(LINE, BG)),
            Rect::new(settings.x - 2, body.y, 1, body.height),
        );
    }
    let fields: Vec<_> = app
        .fields()
        .into_iter()
        .filter(|f| {
            !matches!(
                f,
                Field::Task | Field::Images | Field::Launch | Field::Close
            )
        })
        .collect();
    let columns = if wide { 1 } else { 2 };
    let row_height = if wide { 3 } else { 2 };
    let capacity = (usize::from(settings.height) / row_height * columns).max(1);
    let selected = fields.iter().position(|f| *f == app.focused).unwrap_or(0);
    let start = if selected >= capacity {
        ((selected - capacity) / columns + 1) * columns
    } else {
        0
    };
    for (index, field) in fields.iter().skip(start).take(capacity).enumerate() {
        let col = index % columns;
        let row = index / columns;
        let width = settings.width / columns as u16;
        let rect = Rect::new(
            settings.x + col as u16 * width,
            settings.y + row as u16 * row_height as u16,
            width.saturating_sub(if wide { 0 } else { 2 }),
            (row_height as u16).min(settings.height),
        );
        draw_field(frame, app, *field, rect);
    }
    let footer_y = inner.bottom() - footer_height;
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(style(LINE, BG)),
        Rect::new(area.x, footer_y, area.width, 1),
    );
    let footer = Rect::new(inner.x, footer_y + 1, inner.width, 1);
    let hint = match app.focused {
        Field::Task => "Ctrl+HJKL focus  Ctrl+S launch  Esc close",
        Field::Images => "Ctrl+HJKL focus  Enter inspect  Delete remove",
        Field::Repo => "Ctrl+HJKL focus  Enter choose repo",
        Field::Follow => "Ctrl+HJKL focus  Space toggle",
        _ => "Ctrl+HJKL focus  Enter choose  Ctrl+S launch",
    };
    text(frame, footer, hint, MUTED, BG);
    if footer.width >= 96 {
        let saved = if app.saved { "Draft saved" } else { "Draft" };
        let state = format!(
            "{} · {}",
            saved,
            if app.settings.focus.unwrap_or(app.config.focus) {
                "open workspace"
            } else {
                "background"
            }
        );
        let width = state.len() as u16;
        text(
            frame,
            Rect::new(footer.right() - width, footer.y, width, 1),
            state,
            MUTED,
            BG,
        );
    }
    if !app.message.is_empty() {
        frame.render_widget(
            Paragraph::new(app.message.clone())
                .style(style(RED, BG))
                .wrap(Wrap { trim: true }),
            Rect::new(inner.x, footer_y + 2, inner.width, 2),
        );
    }
    if app.picker.is_some() {
        app.image_placements.clear();
        draw_picker(frame, app);
    }
    if app.image_view {
        draw_image_view(frame, app);
    }
}

fn draw_preview(frame: &mut Frame, app: &mut App, index: usize, area: Rect) {
    if area.is_empty() {
        return;
    }
    app.image_placements.push((index, area));
    if !app
        .graphics
        .as_ref()
        .is_some_and(|g| g.contains(index, &app.settings.attachments[index].path, area))
    {
        app.previews[index].render(frame, area);
    }
}

fn draw_images(frame: &mut Frame, app: &mut App, area: Rect) {
    let count = app.previews.len();
    text(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        format!("Images · {} / {}", app.image_index + 1, count),
        if app.focused == Field::Images {
            BLUE
        } else {
            MUTED
        },
        BG,
    );
    app.hits.push((area, Field::Images));
    if area.height < 4 {
        return;
    }
    let visible = (usize::from(area.width) / 22).clamp(1, 3).min(count);
    let start = app.image_index / visible * visible;
    let width = area.width / visible as u16;
    for (slot, index) in (start..count).take(visible).enumerate() {
        let tile = Rect::new(
            area.x + slot as u16 * width,
            area.y + 1,
            width.saturating_sub(1),
            area.height - 1,
        );
        let selected = index == app.image_index;
        let name = app.settings.attachments[index]
            .name
            .replace(['\n', '\r', '\t'], " ");
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style(if selected { BLUE } else { LINE }, BG))
                .title(Line::styled(
                    format!(" {} · {} ", index + 1, name),
                    style(if selected { BLUE } else { MUTED }, BG),
                )),
            tile,
        );
        let content = tile.inner(Margin::new(1, 1));
        let preview = &mut app.previews[index];
        if preview.error.is_some() {
            text(frame, content, "Image unavailable", RED, BG);
        } else {
            draw_preview(frame, app, index, content);
        }
        app.image_hits.push((tile, index));
    }
}

fn draw_image_view(frame: &mut Frame, app: &mut App) {
    app.image_placements.clear();
    let rect = frame.area().inner(Margin::new(2, 1));
    let attachment = &app.settings.attachments[app.image_index];
    let pixel_preview = app.graphics.as_ref().is_some_and(|g| g.active());
    let preview = &mut app.previews[app.image_index];
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .style(style(INK, BG))
            .border_style(style(BLUE, BG))
            .title(format!(
                " Image {} / {} · {} ",
                app.image_index + 1,
                app.settings.attachments.len(),
                attachment.name.replace(['\n', '\r', '\t'], " ")
            )),
        rect,
    );
    let inner = rect.inner(Margin::new(2, 1));
    text(
        frame,
        Rect::new(inner.x, inner.y, inner.width, 1),
        format!(
            "{} × {} · {} · original attached",
            preview.width,
            preview.height,
            if pixel_preview {
                "Kitty preview"
            } else {
                "terminal thumbnail"
            }
        ),
        MUTED,
        BG,
    );
    let image = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(4),
    );
    if let Some(error) = &preview.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(style(RED, BG))
                .wrap(Wrap { trim: true }),
            image,
        );
    } else {
        draw_preview(frame, app, app.image_index, image);
    }
    text(
        frame,
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        "h/l choose  Delete remove  Esc / Ctrl+H back",
        MUTED,
        BG,
    );
}

fn draw_field(frame: &mut Frame, app: &mut App, field: Field, rect: Rect) {
    let selected = app.focused == field && app.picker.is_none();
    let bg = if selected { SELECT } else { BG };
    frame.render_widget(Block::default().style(style(INK, bg)), rect);
    if matches!(field, Field::Follow | Field::More) {
        text(
            frame,
            Rect::new(rect.x, rect.y + 1, rect.width, 1),
            app.value(field),
            if selected { BLUE } else { INK },
            bg,
        );
    } else {
        text(
            frame,
            Rect::new(rect.x, rect.y, rect.width, 1),
            label(field),
            if selected { BLUE } else { MUTED },
            bg,
        );
        let value = app.value(field);
        text(
            frame,
            Rect::new(rect.x, rect.y + 1, rect.width.saturating_sub(2), 1),
            value,
            INK,
            bg,
        );
        text(
            frame,
            Rect::new(rect.right().saturating_sub(1), rect.y + 1, 1, 1),
            "▾",
            if selected { BLUE } else { MUTED },
            bg,
        );
    }
    app.hits.push((rect, field));
}

fn draw_picker(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let picker = app.picker.as_mut().unwrap();
    let choices = picker.matches();
    let item_height = if picker.field == Field::Repo { 2 } else { 1 };
    let width = 72.min(area.width.saturating_sub(4));
    let height = (choices.len().min(8) as u16 * item_height + 8).min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style(BLUE, PANEL))
            .style(style(INK, PANEL))
            .title(format!(" {} ", label(picker.field))),
        rect,
    );
    let inner = rect.inner(Margin::new(2, 1));
    let prompt = match picker.field {
        Field::Repo => "Search open repos, or enter a path",
        Field::Base => "Choose a base, or enter a ref",
        Field::Branch => "Leave blank for a unique task branch",
        Field::Model => "Model ID or alias · blank for automatic",
        _ if picker.filtering => "Filter",
        _ => "j/k to choose · / to filter",
    };
    text(
        frame,
        Rect::new(inner.x, inner.y, inner.width, 1),
        prompt,
        MUTED,
        PANEL,
    );
    picker.query.set_style(style(INK, SELECT));
    picker.query.set_cursor_line_style(style(INK, SELECT));
    picker.query.set_cursor_style(if picker.filtering {
        style(BG, INK)
    } else {
        style(INK, SELECT)
    });
    frame.render_widget(
        &picker.query,
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
    );
    let available = inner.height.saturating_sub(5) / item_height;
    let start = picker
        .selected
        .saturating_sub(available.saturating_sub(1) as usize);
    for (index, (value, title)) in choices
        .iter()
        .enumerate()
        .skip(start)
        .take(available as usize)
    {
        let row = Rect::new(
            inner.x,
            inner.y + 4 + (index - start) as u16 * item_height,
            inner.width,
            item_height,
        );
        let bg = if index == picker.selected {
            SELECT
        } else {
            PANEL
        };
        text(
            frame,
            Rect::new(row.x, row.y, row.width, 1),
            format!(
                "{} {}",
                if index == picker.selected { "▸" } else { " " },
                title
            ),
            if index == picker.selected { BLUE } else { INK },
            bg,
        );
        if item_height == 2 {
            text(
                frame,
                Rect::new(row.x, row.y + 1, row.width, 1),
                format!("  {value}"),
                MUTED,
                bg,
            );
        }
        app.picker_hits.push((row, index));
    }
    if choices.is_empty() {
        text(
            frame,
            Rect::new(inner.x, inner.y + 4, inner.width, 1),
            if picker.free_text {
                "Enter applies this value"
            } else {
                "No matching options"
            },
            MUTED,
            PANEL,
        );
    }
    text(
        frame,
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        if picker.filtering && !matches!(picker.field, Field::Branch) {
            "Ctrl+J/K choose  Ctrl+L apply  Ctrl+H back"
        } else if matches!(picker.field, Field::Branch) {
            "Ctrl+L / Enter apply   Ctrl+H / Esc back"
        } else {
            "j/k choose  / filter  Ctrl+L apply  Ctrl+H back"
        },
        MUTED,
        PANEL,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Config;
    use ratatui::{backend::TestBackend, Terminal};
    #[test]
    fn attachments_and_preview_fit_small_and_large_terminals() {
        for (width, height) in [(110, 32), (80, 24), (48, 20), (40, 12)] {
            let mut app = App::new(
                Config {
                    attachments: vec![crate::images::Attachment {
                        path: "/nonexistent/composer-test.png".into(),
                        name: "missing.png".into(),
                    }],
                    ..Config::default()
                },
                None,
            );
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for preview in [false, true] {
                app.image_view = preview;
                terminal.draw(|f| draw(f, &mut app)).unwrap();
                assert!(app.editor_rect.width > 0 && app.editor_rect.height > 0);
                for (rect, _) in &app.hits {
                    assert!(rect.right() <= width && rect.bottom() <= height);
                }
            }
        }
    }
    #[test]
    fn compact_and_full_layout_keep_task_and_launch_accessible() {
        for (width, height) in [(110, 32), (80, 24), (48, 20)] {
            let mut app = App::new(
                Config {
                    repo: "/tmp/herdr-worktrunk".into(),
                    default_agent: "codex".into(),
                    task: "Fix auth\n日本語".into(),
                    ..Config::default()
                },
                None,
            );
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            assert!(app.editor_rect.width > 0 && app.editor_rect.height > 0);
            assert!(app.hits.iter().any(|(_, field)| *field == Field::Launch));
            for (rect, _) in &app.hits {
                assert!(rect.right() <= width && rect.bottom() <= height);
            }
        }
    }
}
