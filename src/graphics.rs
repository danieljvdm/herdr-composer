//! Herdr owns Kitty placement, clipping, and replay. The composer sends image
//! layers through the pane API so local and remote clients receive pixel data.
use crate::images::{Attachment, Preview};
use ratatui::layout::Rect;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Placed {
    path: String,
    area: Rect,
    cell: (u32, u32),
    stream: UnixStream,
}

pub struct Graphics {
    socket: PathBuf,
    pane: String,
    cell: Option<(u32, u32)>,
    checked: Option<Instant>,
    placed: BTreeMap<usize, Placed>,
}

impl Graphics {
    pub fn from_env() -> Option<Self> {
        if env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return None;
        }
        Some(Self {
            socket: env::var_os("HERDR_SOCKET_PATH")?.into(),
            pane: env::var("HERDR_PANE_ID").ok()?,
            cell: None,
            checked: None,
            placed: BTreeMap::new(),
        })
    }
    fn request(&self, method: &str, mut params: Value) -> io::Result<(UnixStream, Value)> {
        params["pane_id"] = json!(self.pane);
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        stream.set_write_timeout(Some(Duration::from_millis(500)))?;
        serde_json::to_writer(
            &mut stream,
            &json!({"id":"composer-image","method":method,"params":params}),
        )?;
        stream.write_all(b"\n")?;
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line)?;
        let response: Value = serde_json::from_str(&line)?;
        if let Some(error) = response.get("error") {
            return Err(io::Error::other(error.to_string()));
        }
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| io::Error::other("Missing Herdr response"))?;
        Ok((stream, result))
    }
    pub fn active(&self) -> bool {
        self.cell.is_some() && !self.placed.is_empty()
    }
    pub fn contains(&self, index: usize, path: &str, area: Rect) -> bool {
        self.placed
            .get(&index)
            .is_some_and(|p| p.path == path && p.area == area && Some(p.cell) == self.cell)
    }
    pub fn sync(
        &mut self,
        wanted: &[(usize, Rect)],
        previews: &mut [Preview],
        attachments: &[Attachment],
    ) {
        if wanted.is_empty() && self.placed.is_empty() {
            return;
        }
        if self
            .checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            self.checked = Some(Instant::now());
            self.cell = self
                .request("pane.graphics.info", json!({}))
                .ok()
                .and_then(|(_, info)| {
                    let w = u32::try_from(info["cell_width_px"].as_u64()?).ok()?;
                    let h = u32::try_from(info["cell_height_px"].as_u64()?).ok()?;
                    (w > 0 && h > 0).then_some((w, h))
                });
        }
        self.placed.retain(|index, placed| {
            self.cell.is_some()
                && wanted.iter().any(|(i, _)| i == index)
                && stream_alive(&mut placed.stream)
        });
        let Some(cell) = self.cell else {
            return;
        };
        for &(index, area) in wanted {
            let Some(attachment) = attachments.get(index) else {
                continue;
            };
            if self.contains(index, &attachment.path, area) {
                continue;
            }
            let Some(preview) = previews.get_mut(index) else {
                continue;
            };
            let Some(rect) = fit(area, preview.width, preview.height, cell) else {
                continue;
            };
            let Some((width, height, png)) = preview.png() else {
                continue;
            };
            let result = (|| -> io::Result<()> {
                if !self.placed.contains_key(&index) {
                    let (stream, _) = self.request(
                        "pane.graphics.stream",
                        json!({"layer_id":format!("composer-image-{index}")}),
                    )?;
                    self.placed.insert(
                        index,
                        Placed {
                            path: String::new(),
                            area: Rect::default(),
                            cell,
                            stream,
                        },
                    );
                }
                let placed = self.placed.get_mut(&index).unwrap();
                serde_json::to_writer(
                    &mut placed.stream,
                    &json!({
                        "format":"png","image_width":width,"image_height":height,"data_length":png.len(),
                        "placement":{"viewport_col":rect.x,"viewport_row":rect.y,"grid_cols":rect.width,"grid_rows":rect.height}
                    }),
                )?;
                placed.stream.write_all(b"\n")?;
                placed.stream.write_all(png)?;
                placed.path = attachment.path.clone();
                placed.area = area;
                placed.cell = cell;
                Ok(())
            })();
            if result.is_err() {
                self.placed.remove(&index);
                self.cell = None;
                self.checked = Some(Instant::now());
                break;
            }
        }
    }
}

// Inline streams send no success replies. EOF or an error reply means Herdr
// rejected or closed the layer; dropping the socket also clears its image.
fn stream_alive(stream: &mut UnixStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let live = matches!(stream.read(&mut [0u8;1]),Err(e) if e.kind()==io::ErrorKind::WouldBlock);
    stream.set_nonblocking(false).is_ok() && live
}

fn fit(area: Rect, width: u32, height: u32, cell: (u32, u32)) -> Option<Rect> {
    if area.is_empty() || width == 0 || height == 0 || cell.0 == 0 || cell.1 == 0 {
        return None;
    }
    let scale = (f64::from(area.width) * f64::from(cell.0) / f64::from(width))
        .min(f64::from(area.height) * f64::from(cell.1) / f64::from(height));
    let cols = (f64::from(width) * scale / f64::from(cell.0))
        .round()
        .max(1.0) as u16;
    let rows = (f64::from(height) * scale / f64::from(cell.1))
        .round()
        .max(1.0) as u16;
    Some(Rect::new(
        area.x + (area.width - cols) / 2,
        area.y + (area.height - rows) / 2,
        cols,
        rows,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::net::UnixListener, thread};
    #[test]
    fn pixel_placement_uses_real_cell_dimensions() {
        assert_eq!(
            fit(Rect::new(2, 3, 80, 30), 800, 600, (10, 20)),
            Some(Rect::new(2, 3, 80, 30))
        );
        assert_eq!(
            fit(Rect::new(2, 3, 80, 30), 800, 600, (20, 20)),
            Some(Rect::new(22, 3, 40, 30))
        );
    }
    #[test]
    fn upload_reuse_resize_and_cleanup_follow_visible_images() {
        let root = env::temp_dir().join(format!("composer-graphics-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("api.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap()["method"],
                "pane.graphics.info"
            );
            writeln!(
                stream,
                "{}",
                json!({"result":{"cell_width_px":10,"cell_height_px":20}})
            )
            .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap()["method"],
                "pane.graphics.stream"
            );
            writeln!(stream, "{}", json!({"result":{"type":"ok"}})).unwrap();
            let mut reader = BufReader::new(stream);
            let mut frames = vec![];
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let header: Value = serde_json::from_str(&line).unwrap();
                let mut bytes = vec![0; header["data_length"].as_u64().unwrap() as usize];
                reader.read_exact(&mut bytes).unwrap();
                assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 200);
                frames.push(header["placement"].clone());
            }
            frames
        });
        let image = root.join("image.png");
        image::RgbImage::from_pixel(200, 100, image::Rgb([240, 100, 70]))
            .save(&image)
            .unwrap();
        let attachments = vec![Attachment {
            path: image.to_string_lossy().into_owned(),
            name: "test.png".into(),
        }];
        let mut previews = vec![Preview::load(&image)];
        let mut graphics = Graphics {
            socket,
            pane: "w1:p9".into(),
            cell: None,
            checked: None,
            placed: BTreeMap::new(),
        };
        let area = Rect::new(2, 3, 40, 10);
        graphics.sync(&[(0, area)], &mut previews, &attachments);
        assert!(graphics.contains(0, &attachments[0].path, area));
        graphics.sync(&[(0, area)], &mut previews, &attachments);
        graphics.sync(&[(0, Rect::new(2, 3, 60, 20))], &mut previews, &attachments);
        graphics.sync(&[], &mut previews, &attachments);
        assert!(graphics.placed.is_empty());
        let frames = server.join().unwrap();
        assert_eq!(
            frames.len(),
            2,
            "Unchanged frames must not retransmit pixels"
        );
        assert_ne!(frames[0], frames[1], "Resize must reposition the image");
        fs::remove_dir_all(root).unwrap();
    }
}
