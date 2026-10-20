use anyhow::Context as _;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub fn database_path() -> anyhow::Result<PathBuf> {
    if let Some(data_dir) = std::env::var_os("SMALLER_TALK_DATA_DIR") {
        return Ok(PathBuf::from(data_dir).join("chats.sqlite3"));
    }

    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/SmallerTalk/chats.sqlite3"))
}

pub fn codex_executable() -> PathBuf {
    resolve_codex_executable_from(
        std::env::var_os("SMALLER_TALK_CODEX_BIN"),
        std::env::var_os("PATH"),
        std::env::var_os("HOME"),
    )
}

fn resolve_codex_executable_from(
    explicit: Option<OsString>,
    path: Option<OsString>,
    home: Option<OsString>,
) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }

    if let Some(path) = path {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("codex");
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    if let Some(home) = home {
        let home = PathBuf::from(home);
        for candidate in [
            home.join(".local/bin/codex"),
            PathBuf::from("/opt/homebrew/bin/codex"),
            PathBuf::from("/usr/local/bin/codex"),
        ] {
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
        let nvm_bins = home.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(nvm_bins) {
            let mut candidates = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("bin/codex"))
                .filter(|path| is_executable_file(path))
                .collect::<Vec<_>>();
            candidates.sort();
            if let Some(candidate) = candidates.pop() {
                return candidate;
            }
        }
    }
    PathBuf::from("codex")
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

pub fn claude_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("SMALLER_TALK_CLAUDE_BIN") {
        return path.into();
    }
    let name = "claude";
    if let Some(path) = std::env::var_os("PATH")
        && let Some(found) = std::env::split_paths(&path)
            .map(|p| p.join(name))
            .find(|p| is_executable_file(p))
    {
        return found;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".local/bin/claude");
        if is_executable_file(&path) {
            return path;
        }
    }
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_executable_wins_over_path_search() {
        assert_eq!(
            super::resolve_codex_executable_from(
                Some("/explicit/codex".into()),
                Some("/bin".into()),
                None
            ),
            std::path::PathBuf::from("/explicit/codex")
        );
    }
}
