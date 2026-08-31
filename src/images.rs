use image::{ImageReader, RgbImage};
use ratatui::{layout::Rect, style::Color, Frame};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Cursor, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Attachment {
    pub path: String,
    pub name: String,
}

pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
    pixels: Option<RgbImage>,
    cached: Option<(u16, u16, RgbImage)>,
    png: Option<Vec<u8>>,
}

impl Preview {
    pub fn load(path: &Path) -> Self {
        match read_image(path).and_then(|bytes| decode(&bytes)) {
            Ok(preview) => preview,
            Err(error) => Self {
                width: 0,
                height: 0,
                error: Some(error.to_string()),
                pixels: None,
                cached: None,
                png: None,
            },
        }
    }

    pub fn png(&mut self) -> Option<(u32, u32, &[u8])> {
        let pixels = self.pixels.as_ref()?;
        if self.png.is_none() {
            let mut encoded = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(pixels.clone())
                .write_to(&mut encoded, image::ImageFormat::Png)
                .ok()?;
            self.png = Some(encoded.into_inner());
        }
        Some((pixels.width(), pixels.height(), self.png.as_deref()?))
    }

    // Color half-blocks are part of the normal terminal frame. They survive
    // Herdr popups, reconnects, plain SSH, and terminals without graphics APIs.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let Some(pixels) = &self.pixels else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }
        if self
            .cached
            .as_ref()
            .is_none_or(|(w, h, _)| *w != area.width || *h != area.height)
        {
            let scale = (f64::from(area.width) / f64::from(pixels.width()))
                .min(f64::from(area.height) * 2.0 / f64::from(pixels.height()));
            let width = (f64::from(pixels.width()) * scale).floor().max(1.0) as u32;
            let height = (f64::from(pixels.height()) * scale).floor().max(1.0) as u32;
            let resized = image::imageops::thumbnail(pixels, width, height);
            self.cached = Some((area.width, area.height, resized));
        }
        let pixels = &self.cached.as_ref().unwrap().2;
        let x0 = area.x + (area.width - pixels.width() as u16) / 2;
        let y0 = area.y + (area.height - pixels.height().div_ceil(2) as u16) / 2;
        for y in (0..pixels.height()).step_by(2) {
            for x in 0..pixels.width() {
                let top = pixels.get_pixel(x, y).0;
                let bottom = if y + 1 < pixels.height() {
                    pixels.get_pixel(x, y + 1).0
                } else {
                    [26, 27, 38]
                };
                frame.buffer_mut()[(x0 + x as u16, y0 + (y / 2) as u16)]
                    .set_symbol("▀")
                    .set_fg(Color::Rgb(top[0], top[1], top[2]))
                    .set_bg(Color::Rgb(bottom[0], bottom[1], bottom[2]));
            }
        }
    }
}

fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose an image file.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Image exceeds 64 MiB.".into());
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Preview, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| format!("Cannot decode image: {e}"))?;
    let (width, height) = (decoded.width(), decoded.height());
    let rgba = if width > 2048 || height > 2048 {
        decoded.thumbnail(2048, 2048)
    } else {
        decoded
    }
    .to_rgba8();
    let pixels = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let c = rgba.get_pixel(x, y).0;
        let a = u32::from(c[3]);
        let bg = [26u32, 27, 38];
        image::Rgb(std::array::from_fn(|i| {
            ((u32::from(c[i]) * a + bg[i] * (255 - a)) / 255) as u8
        }))
    });
    Ok(Preview {
        width,
        height,
        error: None,
        pixels: Some(pixels),
        cached: None,
        png: None,
    })
}

pub fn import_file(path: &Path, store: &Path) -> Result<(Attachment, Preview), String> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    import_bytes(&read_image(path)?, &name, store)
}

pub fn import_bytes(
    bytes: &[u8],
    name: &str,
    store: &Path,
) -> Result<(Attachment, Preview), String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Image exceeds 64 MiB.".into());
    }
    let preview = decode(bytes)?;
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let extension = format.extensions_str().first().copied().unwrap_or("png");
    let hash = format!("{:x}", Sha256::digest(bytes));
    crate::storage::private_dir(store).map_err(|e| e.to_string())?;
    let store = fs::canonicalize(store).map_err(|e| e.to_string())?;
    let destination = store.join(format!("{hash}.{extension}"));
    if !destination.is_file() {
        let temporary = store.join(format!("{hash}.{}.tmp", std::process::id()));
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|e| format!("Could not save image: {e}"))?;
    }
    Ok((
        Attachment {
            path: destination.to_string_lossy().into_owned(),
            name: name.into(),
        },
        preview,
    ))
}

fn image_path(token: &str) -> Option<PathBuf> {
    let raw = token.strip_prefix("file://").unwrap_or(token);
    let path = if let Some(rest) = raw.strip_prefix("~/") {
        PathBuf::from(env::var_os("HOME")?).join(rest)
    } else {
        PathBuf::from(raw)
    };
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    ) {
        return None;
    }
    if !path.is_absolute() && !raw.starts_with('.') && !path.is_file() {
        return None;
    }
    Some(path)
}

// Clipboard/file-drop payloads are data, never shell commands. First accept a
// complete path with spaces; otherwise parse quoted/escaped lists like Finder.
pub fn pasted_paths(value: &str) -> Option<Vec<PathBuf>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if Path::new(value).is_file() {
        return image_path(value).map(|p| vec![p]);
    }
    let tokens = shlex::split(value)?;
    if tokens.is_empty() {
        return None;
    }
    tokens.iter().map(|t| image_path(t)).collect()
}

// Herdr --remote transfers the image and pastes a server path. This reader is
// only for a local desktop when Ctrl+V or an empty bracketed paste reaches us.
pub fn clipboard_image() -> Result<Vec<u8>, String> {
    if env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some() {
        return Err(
            "Use herdr --remote for desktop image paste, or paste a path on this host.".into(),
        );
    }
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pngpaste", &["-"])]
    } else {
        &[
            ("wl-paste", &["--no-newline", "--type", "image/png"]),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
        ]
    };
    for (program, args) in commands {
        if let Ok(output) = Command::new(program).args(*args).output() {
            if output.status.success() && !output.stdout.is_empty() {
                return Ok(output.stdout);
            }
        }
    }
    Err("No clipboard image found. Copy an image or paste its file path.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_paths_and_quoted_file_drops_are_data() {
        let paths =
            pasted_paths("/tmp/herdr-clipboard-images-1000/client-4-clipboard-123-0.png").unwrap();
        assert_eq!(paths.len(), 1);
        let paths = pasted_paths("'/tmp/screen one.png' /tmp/screen\\ two.jpg").unwrap();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/screen one.png"),
                PathBuf::from("/tmp/screen two.jpg")
            ]
        );
        assert!(pasted_paths("please fix /tmp/screen.png").is_none());
        assert!(pasted_paths("$(touch /tmp/nope).png").is_none());
    }

    #[test]
    fn imported_original_survives_source_cleanup_and_deduplicates() {
        let root = env::temp_dir().join(format!("composer-image-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("sample image.png");
        image::RgbImage::from_pixel(24, 16, image::Rgb([230, 100, 70]))
            .save(&source)
            .unwrap();
        let original = fs::read(&source).unwrap();
        let (attachment, preview) = import_file(&source, &root.join("saved")).unwrap();
        assert_eq!((preview.width, preview.height), (24, 16));
        let (same, _) = import_file(&source, &root.join("saved")).unwrap();
        assert_eq!(attachment.path, same.path);
        fs::remove_file(source).unwrap();
        assert_eq!(fs::read(&attachment.path).unwrap(), original);
        assert!(Preview::load(Path::new(&attachment.path)).error.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_thumbnail_contains_image_colors() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            16,
            8,
            image::Rgb([240, 80, 50]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        let mut preview = decode(bytes.get_ref()).unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 8)).unwrap();
        terminal.draw(|f| preview.render(f, f.area())).unwrap();
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .filter(|c| c.symbol() == "▀")
                .count(),
            100
        );
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.symbol() == "▀" && c.fg == Color::Rgb(240, 80, 50)));
    }
}
