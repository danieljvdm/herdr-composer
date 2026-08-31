use crate::Result;
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
impl Output {
    pub fn checked(self) -> Result<String> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(format!("command failed: {}{}", self.stderr, self.stdout).into())
        }
    }
    pub fn json(self) -> Result<Value> {
        Ok(serde_json::from_str(&self.checked()?)?)
    }
}
pub fn available(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|p| executable(&p.join(program)))
}
// Pipes are drained concurrently. Output and runtime remain bounded even when a
// command forks a descendant that retains its pipes.
pub fn run(
    argv: &[String],
    cwd: &Path,
    input: Option<&Value>,
    timeout: Duration,
) -> Result<Output> {
    let (program, args) = argv.split_first().ok_or("empty command argv")?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, pipe) in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe.take(1024 * 1024 + 1).read_to_end(&mut bytes);
            let _ = tx.send((index, result, bytes));
        });
    }
    if let Some(value) = input {
        let bytes = serde_json::to_vec(value)?;
        let mut pipe = child.stdin.take().unwrap();
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let start = Instant::now();
    let mut output = [None, None];
    loop {
        while let Ok((index, result, bytes)) = rx.try_recv() {
            result?;
            if bytes.len() > 1024 * 1024 {
                let _ = child.kill();
                let _ = child.wait();
                return Err("command output exceeded 1 MiB; inspect any partial resources".into());
            }
            output[index] = Some(String::from_utf8(bytes)?);
        }
        if let Some(status) = child.try_wait()? {
            if output.iter().all(Option::is_some) {
                return Ok(Output {
                    success: status.success(),
                    stdout: output[0].take().unwrap(),
                    stderr: output[1].take().unwrap(),
                });
            }
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{program} timed out; inspect any partial resources before retrying"
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}
pub fn command(args: &[&str], cwd: &Path) -> Result<String> {
    run(
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        cwd,
        None,
        Duration::from_secs(30),
    )?
    .checked()
    .map(|s| s.trim_end().into())
}
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut argv = vec!["git"];
    argv.extend(args);
    command(&argv, cwd)
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
