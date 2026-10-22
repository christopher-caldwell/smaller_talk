use crate::repository::bounded_read;
use smaller_talk_application::{ExecutionError, TextGenerationInput};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

pub(super) async fn support(executable: &Path) -> Result<(), ExecutionError> {
    let output = timeout(
        Duration::from_secs(6),
        Command::new(executable)
            .args(["exec", "--help"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| ExecutionError::new("Codex isolation check timed out"))?
    .map_err(|e| ExecutionError::with_source("Could not check Codex isolation support", e))?;
    let help = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || [
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "--json",
            "--strict-config",
        ]
        .iter()
        .any(|flag| !help.contains(flag))
    {
        return Err(ExecutionError::new(
            "Update Codex: this version does not advertise the required isolated rewrite controls.",
        ));
    }
    Ok(())
}

pub(super) async fn generate(
    executable: &Path,
    directory: &Path,
    input: &TextGenerationInput,
) -> Result<String, ExecutionError> {
    let instructions = directory.join("rewrite-instructions.md");
    tokio::fs::write(&instructions, &input.instructions)
        .await
        .map_err(|e| ExecutionError::with_source("Could not prepare rewrite instructions", e))?;
    let mut command = Command::new(executable);
    command.args([
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--skip-git-repo-check",
        "--strict-config",
        "--sandbox",
        "read-only",
        "--json",
        "--color",
        "never",
        "--model",
        input.settings.model.as_deref().unwrap_or_default(),
    ]);
    for (key, value) in [
        ("model_instructions_file", serde_json::json!(instructions)),
        ("developer_instructions", serde_json::json!("")),
        (
            "model_reasoning_effort",
            serde_json::json!(input.settings.effort.as_deref().unwrap_or("low")),
        ),
        ("approval_policy", serde_json::json!("never")),
        ("web_search", serde_json::json!("disabled")),
        ("project_doc_max_bytes", serde_json::json!(0)),
        ("skills.include_instructions", serde_json::json!(false)),
        ("agents.enabled", serde_json::json!(false)),
        ("tools.update_plan.enabled", serde_json::json!(false)),
        (
            "tools.experimental_request_user_input.enabled",
            serde_json::json!(false),
        ),
        ("include_environment_context", serde_json::json!(false)),
        ("include_permissions_instructions", serde_json::json!(false)),
        ("include_apps_instructions", serde_json::json!(false)),
        (
            "include_collaboration_mode_instructions",
            serde_json::json!(false),
        ),
        (
            "suppress_unstable_features_warning",
            serde_json::json!(true),
        ),
    ] {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    // Ignore user/project instructions and disable providers of executable capabilities.
    // Strict config makes an incompatible CLI fail instead of ignoring these controls.
    for feature in [
        "shell_tool",
        "unified_exec",
        "shell_snapshot",
        "apps",
        "plugins",
        "hooks",
        "multi_agent",
        "multi_agent_v2",
        "code_mode",
        "code_mode_host",
        "browser_use",
        "computer_use",
        "image_generation",
        "view_image",
        "memories",
        "skill_search",
        "tool_suggest",
        "sleep_tool",
        "goals",
        "workspace_dependencies",
    ] {
        command.arg("-c").arg(format!("features.{feature}=false"));
    }
    command
        .args(["-c", "features.skip_host_skill_discovery=true", "-"])
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A nested invocation must not attach to the calling app's tools/session.
    // Retain CODEX_HOME so authentication stays owned by the installed CLI.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CODEX_") && key != "CODEX_HOME" {
            command.env_remove(key);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| ExecutionError::with_source("Could not start isolated Codex rewrite", e))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let prompt = serde_json::to_vec(&serde_json::json!({"answer": input.text}))
        .map_err(|e| ExecutionError::with_source("Could not encode rewrite input", e))?;
    let write = async move {
        stdin.write_all(&prompt).await?;
        stdin.shutdown().await
    };
    let (written, output, diagnostic, status) = tokio::join!(
        write,
        bounded_read(stdout, 1024 * 1024),
        bounded_read(stderr, 4096),
        child.wait()
    );
    written.map_err(|e| ExecutionError::with_source("Could not send rewrite input", e))?;
    let (output, truncated) = output
        .map_err(|e| ExecutionError::with_source("Could not read Codex rewrite output", e))?;
    diagnostic
        .map_err(|e| ExecutionError::with_source("Could not drain Codex rewrite diagnostics", e))?;
    let status =
        status.map_err(|e| ExecutionError::with_source("Could not wait for Codex rewrite", e))?;
    if !status.success() || truncated {
        return Err(ExecutionError::new(
            "Codex rewrite failed or exceeded its output limit. Check CLI sign-in and model access, then retry.",
        ));
    }
    answer(&output)
}

fn answer(output: &[u8]) -> Result<String, ExecutionError> {
    let mut completed = false;
    let mut final_text = None;
    for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let frame: serde_json::Value = serde_json::from_slice(line)
            .map_err(|e| ExecutionError::with_source("Codex rewrite returned invalid output", e))?;
        match frame["type"].as_str() {
            Some("turn.completed") => completed = true,
            Some("turn.failed" | "error") => {
                return Err(ExecutionError::new("Codex could not finish the rewrite"));
            }
            Some("item.started" | "item.updated" | "item.completed") => {
                match frame["item"]["type"].as_str() {
                    Some("agent_message") if frame["type"] == "item.completed" => {
                        final_text = frame["item"]["text"].as_str().map(str::to_owned);
                    }
                    Some("agent_message" | "reasoning" | "error") => {}
                    _ => {
                        return Err(ExecutionError::new(
                            "Codex attempted a tool action during text-only rewriting. Shortening was rejected.",
                        ));
                    }
                }
            }
            Some("thread.started" | "turn.started") => {}
            _ => {
                return Err(ExecutionError::new(
                    "Codex returned an unsupported rewrite event. Update the app and retry.",
                ));
            }
        }
    }
    final_text
        .filter(|s| completed && !s.trim().is_empty())
        .ok_or_else(|| ExecutionError::new("Codex did not return a completed rewrite answer"))
}
