use async_trait::async_trait;
use smaller_talk_application::{ExecutionError, RepositoryReader};
use smaller_talk_domain::*;
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};

const PREVIEW_LIMIT: usize = 1024 * 1024;
const STATUS_LIMIT: usize = 8 * 1024 * 1024;
pub struct GitRepositoryReader {
    git: PathBuf,
    gh: PathBuf,
}
impl GitRepositoryReader {
    pub fn new(git: PathBuf, gh: PathBuf) -> Self {
        Self { git, gh }
    }
}

pub(crate) async fn bounded_read(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let retain = count.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..retain]);
        truncated |= retain < count;
    }
    Ok((output, truncated))
}
struct GitOutput {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
    truncated: bool,
}
impl GitRepositoryReader {
    async fn git(
        &self,
        folder: &Path,
        args: &[OsString],
        limit: usize,
    ) -> Result<GitOutput, ExecutionError> {
        let mut command = Command::new(&self.git);
        command
            .arg("--no-pager")
            .arg("--literal-pathspecs")
            .arg("-c")
            .arg("core.quotepath=false")
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .current_dir(folder)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        timeout(Duration::from_secs(10), async {
            let mut child = command
                .spawn()
                .map_err(|e| ExecutionError::with_source("Could not run Git", e))?;
            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let (stdout, stderr, status) = tokio::join!(
                bounded_read(stdout, limit),
                bounded_read(stderr, 4096),
                child.wait()
            );
            let (stdout, truncated) =
                stdout.map_err(|e| ExecutionError::with_source("Could not read Git output", e))?;
            let (stderr, _) = stderr
                .map_err(|e| ExecutionError::with_source("Could not read Git diagnostics", e))?;
            let status =
                status.map_err(|e| ExecutionError::with_source("Could not wait for Git", e))?;
            Ok(GitOutput {
                ok: status.success(),
                stdout,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                truncated,
            })
        })
        .await
        .map_err(|_| ExecutionError::new("Git review timed out. Refresh to try again."))?
    }
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec()).into()
}
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    String::from_utf8_lossy(bytes).into_owned().into()
}
fn checked(result: GitOutput) -> Result<GitOutput, ExecutionError> {
    if result.ok {
        Ok(result)
    } else {
        Err(ExecutionError::new(format!(
            "Could not read repository changes: {}",
            result.stderr.trim()
        )))
    }
}
fn parse_status(
    bytes: &[u8],
    scope: DiffScope,
) -> Result<(String, Vec<ChangedFile>), ExecutionError> {
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut branch = String::from("Unknown branch");
    let mut oid = String::new();
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if let Some(value) = record.strip_prefix(b"# branch.head ") {
            branch = String::from_utf8_lossy(value).into_owned();
            continue;
        }
        if let Some(value) = record.strip_prefix(b"# branch.oid ") {
            oid = String::from_utf8_lossy(value).into_owned();
            continue;
        }
        if record.starts_with(b"#") || record.starts_with(b"!") {
            continue;
        }
        if let Some(path) = record.strip_prefix(b"? ") {
            if scope == DiffScope::Unstaged {
                files.push(ChangedFile {
                    path: path_from_bytes(path),
                    previous_path: None,
                    status: "New".into(),
                    additions: None,
                    deletions: None,
                    untracked: true,
                });
            }
            continue;
        }
        let fields = match record.first() {
            Some(b'1') => 9,
            Some(b'2') => 10,
            Some(b'u') => 11,
            _ => {
                return Err(ExecutionError::new(
                    "Git returned an unsupported status record",
                ));
            }
        };
        let parts: Vec<_> = record.splitn(fields, |b| *b == b' ').collect();
        if parts.len() != fields || parts[1].len() != 2 {
            return Err(ExecutionError::new(
                "Git returned an incomplete status record",
            ));
        }
        let previous_path = if record[0] == b'2' {
            Some(path_from_bytes(records.next().ok_or_else(|| {
                ExecutionError::new("Git rename has no original path")
            })?))
        } else {
            None
        };
        let code = parts[1][if scope == DiffScope::Staged { 0 } else { 1 }];
        if code == b'.' {
            continue;
        }
        files.push(ChangedFile {
            path: path_from_bytes(parts[fields - 1]),
            previous_path,
            status: if record[0] == b'u' {
                "Conflict"
            } else {
                match code {
                    b'A' => "Added",
                    b'D' => "Deleted",
                    b'R' => "Renamed",
                    b'T' => "Type changed",
                    b'C' => "Copied",
                    _ => "Modified",
                }
            }
            .into(),
            additions: Some(0),
            deletions: Some(0),
            untracked: false,
        });
    }
    if branch == "(detached)" {
        branch = format!("Detached · {}", oid.chars().take(8).collect::<String>());
    }
    Ok((branch, files))
}
fn parse_numstat(bytes: &[u8]) -> HashMap<PathBuf, (Option<u64>, Option<u64>)> {
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut counts = HashMap::new();
    while let Some(record) = records.next() {
        let parts: Vec<_> = record.splitn(3, |b| *b == b'\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let path = if parts[2].is_empty() {
            let _ = records.next();
            match records.next() {
                Some(path) => path,
                None => break,
            }
        } else {
            parts[2]
        };
        let number = |b: &[u8]| std::str::from_utf8(b).ok().and_then(|s| s.parse().ok());
        counts.insert(path_from_bytes(path), (number(parts[0]), number(parts[1])));
    }
    counts
}
#[async_trait]
impl RepositoryReader for GitRepositoryReader {
    async fn status(
        &self,
        folder: &Path,
        scope: DiffScope,
    ) -> Result<Option<RepositoryStatus>, ExecutionError> {
        let root = self
            .git(
                folder,
                &args(&["rev-parse", "--show-toplevel"]),
                STATUS_LIMIT,
            )
            .await?;
        if !root.ok {
            if root.stderr.contains("not a git repository")
                || root.stderr.contains("must be run in a work tree")
            {
                return Ok(None);
            }
            return Err(ExecutionError::new(format!(
                "Could not detect repository: {}",
                root.stderr.trim()
            )));
        }
        let root_bytes = root.stdout.strip_suffix(b"\n").unwrap_or(&root.stdout);
        let root = path_from_bytes(root_bytes);
        let result = checked(
            self.git(
                &root,
                &args(&[
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--branch",
                    "--untracked-files=all",
                ]),
                STATUS_LIMIT,
            )
            .await?,
        )?;
        if result.truncated {
            return Err(ExecutionError::new(
                "Repository status exceeds the 8 MiB review limit",
            ));
        }
        let (branch, mut files) = parse_status(&result.stdout, scope)?;
        let mut command = args(&["diff", "--no-ext-diff", "--no-textconv", "--numstat", "-z"]);
        if scope == DiffScope::Staged {
            command.push("--cached".into());
        }
        let result = checked(self.git(&root, &command, STATUS_LIMIT).await?)?;
        if result.truncated {
            return Err(ExecutionError::new(
                "Repository statistics exceed the review limit",
            ));
        }
        let counts = parse_numstat(&result.stdout);
        for file in &mut files {
            if let Some((additions, deletions)) = counts.get(&file.path) {
                file.additions = *additions;
                file.deletions = *deletions;
            }
            if file.untracked
                && let Ok(meta) = tokio::fs::symlink_metadata(root.join(&file.path)).await
            {
                if meta.file_type().is_symlink() {
                    file.additions = Some(1);
                    file.deletions = Some(0);
                } else if meta.is_file()
                    && meta.len() <= PREVIEW_LIMIT as u64
                    && let Ok(mut handle) = tokio::fs::File::open(root.join(&file.path)).await
                {
                    let mut bytes = Vec::new();
                    if (&mut handle)
                        .take(PREVIEW_LIMIT as u64 + 1)
                        .read_to_end(&mut bytes)
                        .await
                        .is_ok()
                        && bytes.len() <= PREVIEW_LIMIT
                        && !bytes.contains(&0)
                    {
                        file.additions = Some(
                            bytes.split(|b| *b == b'\n').count() as u64
                                - u64::from(bytes.ends_with(b"\n"))
                                - u64::from(bytes.is_empty()),
                        );
                        file.deletions = Some(0);
                    }
                }
            }
        }
        Ok(Some(RepositoryStatus {
            root,
            branch,
            files,
            scope,
        }))
    }
    async fn diff(
        &self,
        root: &Path,
        file: &ChangedFile,
        scope: DiffScope,
    ) -> Result<FileDiff, ExecutionError> {
        for path in std::iter::once(&file.path).chain(file.previous_path.iter()) {
            if path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(ExecutionError::new("Invalid repository-relative file path"));
            }
        }
        let (bytes, truncated) = if file.untracked {
            let path = root.join(&file.path);
            let meta = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|e| ExecutionError::with_source("The file is no longer available", e))?;
            if meta.file_type().is_symlink() {
                let target = tokio::fs::read_link(path)
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not read symbolic link", e))?;
                (target.display().to_string().into_bytes(), false)
            } else if !meta.is_file() {
                return Ok(FileDiff {
                    text: "Special file · preview unavailable".into(),
                    truncated: false,
                    binary: false,
                });
            } else {
                let mut bytes = Vec::new();
                tokio::fs::File::open(path)
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not open file", e))?
                    .take(PREVIEW_LIMIT as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not read file", e))?;
                let truncated = bytes.len() > PREVIEW_LIMIT;
                bytes.truncate(PREVIEW_LIMIT);
                (bytes, truncated)
            }
        } else {
            let mut command = args(&["diff", "--no-ext-diff", "--no-textconv", "--no-color"]);
            if scope == DiffScope::Staged {
                command.push("--cached".into());
            }
            command.push("--".into());
            command.push(file.path.as_os_str().to_owned());
            if let Some(previous) = &file.previous_path {
                command.push(previous.as_os_str().to_owned());
            }
            let result = checked(self.git(root, &command, PREVIEW_LIMIT).await?)?;
            (result.stdout, result.truncated)
        };
        // Git's binary diagnostics are metadata lines in the diff header, not an
        // arbitrary substring test: a text hunk may legitimately contain these words.
        let invalid_utf8 = match std::str::from_utf8(&bytes) {
            Ok(_) => false,
            Err(error) => !(truncated && error.error_len().is_none()),
        };
        let binary = bytes.contains(&0)
            || invalid_utf8
            || (!file.untracked
                && String::from_utf8_lossy(&bytes).lines().any(|line| {
                    (line.starts_with("Binary files ") && line.ends_with(" differ"))
                        || line == "GIT binary patch"
                }));
        let text = if binary {
            "Binary file · preview unavailable".into()
        } else if file.untracked {
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(|l| format!("+{l}\n"))
                .collect::<String>()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let mut text = text;
        let mut end = text.len().min(PREVIEW_LIMIT);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = truncated || end < text.len();
        text.truncate(end);
        Ok(FileDiff {
            text,
            truncated,
            binary,
        })
    }
    async fn pull_request(&self, root: PathBuf) -> Option<String> {
        let mut child = Command::new(&self.gh)
            .args(["pr", "view", "--json", "url"])
            .current_dir(root)
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (output, status) = timeout(Duration::from_secs(8), async {
            tokio::join!(bounded_read(stdout, 8192), child.wait())
        })
        .await
        .ok()?;
        if !status.ok()?.success() {
            return None;
        }
        let (output, truncated) = output.ok()?;
        if truncated {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&output).ok()?;
        let url = value["url"].as_str()?;
        url.starts_with("https://").then(|| url.to_owned())
    }
}
