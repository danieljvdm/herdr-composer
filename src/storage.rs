use crate::{request::Draft, Result, VERSION};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub fn private_dir(path: &Path) -> Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true).mode(0o700);
    b.create(path)?;
    Ok(())
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    private_dir(path.parent().ok_or("path has no parent")?)?;
    let temp = path.with_extension(format!("{}.tmp", crate::request::launch_id()));
    let result = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer(&mut f, value)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    private_dir(path.parent().ok_or("path has no parent")?)?;
    let temp = path.with_extension(format!("{}.tmp", crate::request::launch_id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp, path)?;
        File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}
pub fn lock(path: &Path) -> Result<File> {
    private_dir(path.parent().ok_or("lock has no parent")?)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    f.try_lock()
        .map_err(|_| "record is in use by another composer process")?;
    Ok(f)
}
pub fn draft_path(state: &Path, context: Option<&Path>) -> PathBuf {
    let key = context
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "global".into());
    state
        .join("drafts")
        .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}
pub fn load_draft(path: &Path) -> Result<Option<Draft>> {
    if !path.exists() {
        return Ok(None);
    }
    let d: Draft = read_json(path)?;
    if d.version != VERSION {
        return Err("unsupported draft version".into());
    }
    Ok(Some(d))
}
pub fn save_draft(path: &Path, draft: &mut Draft) -> Result<()> {
    let _lock = lock(&path.with_extension("lock"))?;
    let revision = load_draft(path)?.map_or(0, |d| d.revision);
    if revision != draft.revision {
        return Err("A newer draft exists. Close and reopen to recover it; this editor has not overwritten it.".into());
    }
    draft.revision += 1;
    write_json(path, draft)
}
pub fn clear_draft(path: &Path, revision: u64) -> Result<()> {
    let _lock = lock(&path.with_extension("lock"))?;
    if let Some(mut draft) = load_draft(path)? {
        if draft.revision == revision {
            draft.task.clear();
            draft.attachments.clear();
            draft.revision += 1;
            write_json(path, &draft)?;
        }
    }
    Ok(())
}
