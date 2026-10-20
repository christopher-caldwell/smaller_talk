use smaller_talk_application::RepositoryReader;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::GitRepositoryReader;
use std::{path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
fn run(root: &Path, args: &[&str]) {
    let out = git(root, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn reader() -> GitRepositoryReader {
    GitRepositoryReader::new("git".into(), "missing-gh-fixture".into())
}
fn init(root: &Path) {
    run(root, &["init", "-b", "main"]);
}
fn commit(root: &Path) {
    run(root, &["add", "."]);
    run(root, &["commit", "-m", "fixture"]);
}
#[tokio::test]
async fn unborn_staged_unstaged_unusual_names_and_index_are_preserved() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    std::fs::create_dir(root.join("nested")).unwrap();
    let name = "space\tand\nÉ🙂.txt";
    std::fs::write(root.join(name), "first\n").unwrap();
    let state = reader()
        .status(&root.join("nested"), DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.root, root.canonicalize().unwrap());
    assert_eq!(state.files[0].path, Path::new(name));
    assert_eq!(state.files[0].additions, Some(1));
    run(root, &["add", "."]);
    let state = reader()
        .status(root, DiffScope::Staged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.files.len(), 1);
    assert!(
        reader()
            .diff(root, &state.files[0], DiffScope::Staged)
            .await
            .unwrap()
            .text
            .contains("+first")
    );
    run(root, &["commit", "-m", "initial"]);
    std::fs::write(root.join(name), "first\nsecond\n").unwrap();
    run(root, &["add", "."]);
    std::fs::write(root.join(name), "first\nsecond\nthird\n").unwrap();
    let before = std::fs::read(root.join(".git/index")).unwrap();
    for scope in [DiffScope::Staged, DiffScope::Unstaged] {
        let state = reader().status(root, scope).await.unwrap().unwrap();
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].additions, Some(1));
        assert_eq!(state.files[0].deletions, Some(0));
        reader().diff(root, &state.files[0], scope).await.unwrap();
    }
    assert_eq!(before, std::fs::read(root.join(".git/index")).unwrap());
}
#[tokio::test]
async fn renames_deletions_binary_and_large_previews() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    std::fs::write(root.join("old"), "original\n").unwrap();
    std::fs::write(root.join("delete"), "remove\n").unwrap();
    commit(root);
    run(root, &["mv", "old", "new"]);
    std::fs::remove_file(root.join("delete")).unwrap();
    std::fs::write(root.join("binary"), [0, 1, 2]).unwrap();
    std::fs::write(root.join("large"), "x".repeat(1024 * 1024 + 100)).unwrap();
    let staged = reader()
        .status(root, DiffScope::Staged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        staged.files[0].previous_path.as_deref(),
        Some(Path::new("old"))
    );
    assert!(
        reader()
            .diff(root, &staged.files[0], DiffScope::Staged)
            .await
            .unwrap()
            .text
            .contains("rename")
    );
    let state = reader()
        .status(root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    assert!(state.files.iter().any(|f| f.status == "Deleted"));
    let binary = state
        .files
        .iter()
        .find(|f| f.path == Path::new("binary"))
        .unwrap();
    assert_eq!(binary.additions, None);
    assert!(
        reader()
            .diff(root, binary, DiffScope::Unstaged)
            .await
            .unwrap()
            .binary
    );
    let large = state
        .files
        .iter()
        .find(|f| f.path == Path::new("large"))
        .unwrap();
    let diff = reader()
        .diff(root, large, DiffScope::Unstaged)
        .await
        .unwrap();
    assert!(diff.truncated);
    assert!(diff.text.len() <= 1024 * 1024);
}
#[tokio::test]
async fn linked_worktrees_detached_head_and_missing_pr_tool_are_supported() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    init(&root);
    std::fs::write(root.join("file"), "base\n").unwrap();
    commit(&root);
    let worktree = t.path().join("linked");
    run(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            worktree.to_str().unwrap(),
        ],
    );
    std::fs::write(worktree.join("file"), "changed\n").unwrap();
    let state = reader()
        .status(&worktree, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.branch, "linked");
    assert_eq!(state.root, worktree.canonicalize().unwrap());
    run(&root, &["checkout", "--detach"]);
    let state = reader()
        .status(&root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    assert!(state.branch.starts_with("Detached"));
    assert_eq!(reader().pull_request(root).await, None);
}
#[tokio::test]
async fn conflicts_and_external_diff_drivers_do_not_execute_helpers() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    std::fs::write(root.join("file"), "base\n").unwrap();
    commit(root);
    run(root, &["checkout", "-b", "other"]);
    std::fs::write(root.join("file"), "other\n").unwrap();
    commit(root);
    run(root, &["checkout", "main"]);
    std::fs::write(root.join("file"), "main\n").unwrap();
    commit(root);
    assert!(!git(root, &["merge", "other"]).status.success());
    let state = reader()
        .status(root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.files[0].status, "Conflict");
    run(
        root,
        &["config", "diff.external", "sh -c 'touch SHOULD_NOT_EXIST'"],
    );
    run(
        root,
        &[
            "config",
            "core.fsmonitor",
            "sh -c 'touch MONITOR_SHOULD_NOT_EXIST'",
        ],
    );
    reader().status(root, DiffScope::Unstaged).await.unwrap();
    reader()
        .diff(root, &state.files[0], DiffScope::Unstaged)
        .await
        .unwrap();
    assert!(!root.join("SHOULD_NOT_EXIST").exists());
    assert!(!root.join("MONITOR_SHOULD_NOT_EXIST").exists());
}

#[tokio::test]
async fn text_containing_git_binary_diagnostic_words_remains_visible() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    std::fs::write(root.join("notes.txt"), "Binary files are not previewed.\n").unwrap();
    let state = reader()
        .status(root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    let diff = reader()
        .diff(root, &state.files[0], DiffScope::Unstaged)
        .await
        .unwrap();
    assert!(!diff.binary);
    assert!(diff.text.contains("+Binary files are not previewed."));
}

#[tokio::test]
async fn truncated_utf8_text_remains_a_text_preview() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    let mut contents = vec![b'x'; 1024 * 1024 - 1];
    contents.push(0xF0); // First byte of a four-byte character, cut by the preview cap.
    contents.push(0x9F);
    std::fs::write(root.join("truncated.txt"), contents).unwrap();

    let state = reader()
        .status(root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    let diff = reader()
        .diff(root, &state.files[0], DiffScope::Unstaged)
        .await
        .unwrap();

    assert!(!diff.binary);
    assert!(diff.truncated);
    assert!(diff.text.contains("x"));
    assert!(!diff.text.contains("Binary file"));
}
#[tokio::test]
async fn non_repository_and_symlinks_are_safe() {
    let t = tempfile::tempdir().unwrap();
    assert!(
        reader()
            .status(t.path(), DiffScope::Unstaged)
            .await
            .unwrap()
            .is_none()
    );
    init(t.path());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/outside/private", t.path().join("link")).unwrap();
        let state = reader()
            .status(t.path(), DiffScope::Unstaged)
            .await
            .unwrap()
            .unwrap();
        let diff = reader()
            .diff(t.path(), &state.files[0], DiffScope::Unstaged)
            .await
            .unwrap();
        assert!(diff.text.contains("/outside/private"));
    }
}

#[tokio::test]
async fn diff_paths_are_literal_even_when_names_look_like_git_pathspecs() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    init(root);
    for name in ["glob[ab].txt", "globa.txt", ":(glob)*.txt"] {
        std::fs::write(root.join(name), "before\n").unwrap();
    }
    commit(root);
    for name in ["glob[ab].txt", "globa.txt", ":(glob)*.txt"] {
        std::fs::write(root.join(name), format!("changed {name}\n")).unwrap();
    }
    let state = reader()
        .status(root, DiffScope::Unstaged)
        .await
        .unwrap()
        .unwrap();
    for name in ["glob[ab].txt", ":(glob)*.txt"] {
        let file = state
            .files
            .iter()
            .find(|f| f.path == Path::new(name))
            .unwrap();
        let diff = reader()
            .diff(root, file, DiffScope::Unstaged)
            .await
            .unwrap();
        assert!(diff.text.contains(&format!("+changed {name}")));
        assert!(!diff.text.contains("+changed globa.txt"));
    }
}
