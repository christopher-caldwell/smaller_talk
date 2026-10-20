use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiffScope {
    #[default]
    Unstaged,
    Staged,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub previous_path: Option<PathBuf>,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub untracked: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryStatus {
    pub root: PathBuf,
    pub branch: String,
    pub files: Vec<ChangedFile>,
    pub scope: DiffScope,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiff {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}
