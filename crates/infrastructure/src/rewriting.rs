use crate::{AgentRouter, repository::bounded_read};
use async_trait::async_trait;
use smaller_talk_application::{ExecutionError, TextGenerationInput, TextGenerator};
use smaller_talk_domain::Provider;
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

mod codex;

/// Standalone text work never shares an execution session or its project directory.
pub struct CliTextGenerator {
    router: Arc<AgentRouter>,
    scratch: PathBuf,
    managed_policy_paths: Vec<PathBuf>,
}
impl CliTextGenerator {
    pub fn new(
        router: Arc<AgentRouter>,
        scratch: PathBuf,
        managed_policy_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            router,
            scratch,
            managed_policy_paths,
        }
    }
}
#[async_trait]
impl TextGenerator for CliTextGenerator {
    async fn support(&self, provider: Provider) -> Result<(), ExecutionError> {
        let executable = self.router.executable_for(provider)?;
        if provider == Provider::Codex {
            return codex::support(&executable).await;
        }
        if self.managed_policy_paths.iter().any(|p| p.exists()) {
            return Err(ExecutionError::new(
                "Shortening is unavailable with managed Claude policy files because their hooks cannot be disabled by the app.",
            ));
        }
        let output = timeout(
            Duration::from_secs(6),
            Command::new(executable)
                .arg("--help")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| ExecutionError::new("CLI isolation check timed out"))?
        .map_err(|e| ExecutionError::with_source("Could not check CLI isolation support", e))?;
        let help = String::from_utf8_lossy(&output.stdout);
        let required = [
            "--safe-mode",
            "--tools",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--setting-sources",
            "--system-prompt",
        ];
        if !output.status.success() || required.iter().any(|flag| !help.contains(flag)) {
            return Err(ExecutionError::new(
                "Update Claude Code: this version does not advertise all required text-only isolation controls.",
            ));
        }
        Ok(())
    }
    async fn generate(&self, input: TextGenerationInput) -> Result<String, ExecutionError> {
        self.support(input.settings.provider).await?;
        let executable = self.router.executable_for(input.settings.provider)?;
        tokio::fs::create_dir_all(&self.scratch)
            .await
            .map_err(|e| {
                ExecutionError::with_source("Could not prepare isolated rewrite storage", e)
            })?;
        let directory = tempfile::Builder::new()
            .prefix("answer-")
            .tempdir_in(&self.scratch)
            .map_err(|e| {
                ExecutionError::with_source("Could not create isolated rewrite directory", e)
            })?;
        let model = input
            .settings
            .model
            .as_deref()
            .ok_or_else(|| ExecutionError::new("Choose a rewrite model in Settings"))?;
        if input.settings.provider == Provider::Codex {
            return codex::generate(&executable, directory.path(), &input).await;
        }
        let mut command = Command::new(executable);
        command
            .args([
                "-p",
                "--safe-mode",
                "--tools",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--disable-slash-commands",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--permission-mode",
                "dontAsk",
                "--max-turns",
                "1",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-chrome",
                "--system-prompt",
            ])
            .arg(input.instructions)
            .arg("--model")
            .arg(model)
            .arg("--settings")
            .arg("{\"disableAllHooks\":true,\"autoMemoryEnabled\":false}")
            .current_dir(directory.path())
            .env("CLAUDE_CODE_DISABLE_FILE_MENTIONS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| {
            ExecutionError::with_source("Could not start isolated text generation", e)
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let prompt = serde_json::to_vec(&serde_json::json!({"answer":input.text}))
            .map_err(|e| ExecutionError::with_source("Could not encode rewrite input", e))?;
        let write = async move {
            let mut stdin = stdin;
            stdin.write_all(&prompt).await?;
            stdin.shutdown().await
        };
        let (written, stdout, stderr, status) = tokio::join!(
            write,
            bounded_read(stdout, 1024 * 1024),
            bounded_read(stderr, 4096),
            child.wait()
        );
        written.map_err(|e| ExecutionError::with_source("Could not send rewrite input", e))?;
        let (stdout, truncated) =
            stdout.map_err(|e| ExecutionError::with_source("Could not read rewrite output", e))?;
        let _ = stderr
            .map_err(|e| ExecutionError::with_source("Could not drain rewrite diagnostics", e))?;
        let status = status
            .map_err(|e| ExecutionError::with_source("Could not wait for rewrite process", e))?;
        if !status.success() || truncated {
            return Err(ExecutionError::new(
                "Rewrite generation failed or exceeded its output limit. Check CLI sign-in and retry.",
            ));
        }
        let mut verified = false;
        let mut final_text = None;
        for line in stdout.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let frame: serde_json::Value = serde_json::from_slice(line).map_err(|e| {
                ExecutionError::with_source("The rewrite CLI returned invalid output", e)
            })?;
            if frame["type"] == "system" && frame["subtype"] == "init" {
                verified = frame["tools"].as_array().is_some_and(|a| a.is_empty())
                    && frame["mcp_servers"]
                        .as_array()
                        .is_some_and(|a| a.is_empty());
            }
            if matches!(
                frame["subtype"].as_str(),
                Some("hook_started" | "hook_response")
            ) || frame["message"]["content"]
                .as_array()
                .is_some_and(|a| a.iter().any(|b| b["type"] == "tool_use"))
            {
                return Err(ExecutionError::new(
                    "The CLI did not honor text-only isolation. Shortening was rejected.",
                ));
            }
            if frame["type"] == "result" {
                if frame["is_error"] == true {
                    return Err(ExecutionError::new(
                        "The rewrite model could not finish. Check CLI sign-in or model access.",
                    ));
                }
                final_text = frame["result"].as_str().map(str::to_owned);
            }
        }
        if !verified {
            return Err(ExecutionError::new(
                "The CLI did not confirm an empty tool and MCP catalog. Shortening was rejected.",
            ));
        }
        final_text
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| ExecutionError::new("The rewrite model returned no answer"))
    }
}
