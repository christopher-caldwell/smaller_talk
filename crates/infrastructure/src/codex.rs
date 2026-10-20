use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use serde_json::{Value, json};
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest,
};
use smaller_talk_domain::*;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::mpsc,
    time::timeout,
};

const ADAPTER_KEY: &str = "codex";
const CLIENT_NAME: &str = "smaller_talk";
const CLIENT_TITLE: &str = "Smaller Talk";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct CodexAdapter {
    executable: PathBuf,
}

impl CodexAdapter {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    async fn discover_models(&self) -> Result<ProviderCapabilities, ExecutionError> {
        timeout(Duration::from_secs(25), async {
            let mut child = Command::new(&self.executable).args(["app-server", "--stdio"])
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)
                .spawn().map_err(|e| ExecutionError::with_source("Could not start Codex. Install/sign in to Codex or set SMALLER_TALK_CODEX_BIN.", e))?;
            let mut stdin = child.stdin.take().ok_or_else(|| ExecutionError::new("Codex stdin is unavailable"))?;
            let mut stdout = BufReader::new(child.stdout.take().ok_or_else(|| ExecutionError::new("Codex stdout is unavailable"))?);
            let stderr = child.stderr.take().ok_or_else(|| ExecutionError::new("Codex stderr is unavailable"))?;
            let diagnostic = tokio::spawn(read_stderr(stderr));
            send_request(&mut stdin, 10, "initialize", json!({ "clientInfo": { "name": CLIENT_NAME, "title": CLIENT_TITLE, "version": CLIENT_VERSION }, "capabilities": { "experimentalApi": true } })).await?;
            read_response(&mut stdout, &mut stdin, 10).await?;
            send_notification(&mut stdin, "initialized", json!({})).await?;
            let mut models = Vec::new(); let mut cursor = Value::Null;
            loop {
                send_request(&mut stdin, 11, "model/list", json!({ "cursor": cursor, "limit": 100 })).await?;
                let result = read_response(&mut stdout, &mut stdin, 11).await?;
                for model in result["data"].as_array().into_iter().flatten() {
                    if model["hidden"].as_bool() == Some(true) { continue; }
                    let id = model["model"].as_str().or_else(|| model["id"].as_str()).ok_or_else(|| ExecutionError::new("Codex returned a model without an identifier"))?;
                    models.push(ModelCapability {
                        id: id.into(), label: model["displayName"].as_str().unwrap_or(id).into(),
                        efforts: model["supportedReasoningEfforts"].as_array().into_iter().flatten().filter_map(|e| e["reasoningEffort"].as_str().or_else(|| e.as_str()).map(str::to_owned)).collect(),
                        default_effort: model["defaultReasoningEffort"].as_str().map(str::to_owned),
                        images: model["inputModalities"].as_array().is_none_or(|m| m.iter().any(|m| m.as_str() == Some("image"))),
                        is_default: model["isDefault"].as_bool().unwrap_or(false),
                    });
                }
                let next = result["nextCursor"].clone();
                if next.is_null() { break; }
                if next == cursor { return Err(ExecutionError::new("Codex model discovery repeated a page")); }
                cursor = next;
            }
            drop(stdin);
            if timeout(Duration::from_secs(2), child.wait()).await.is_err() { let _ = child.kill().await; let _ = child.wait().await; }
            let _ = diagnostic.await;
            if models.is_empty() { return Err(ExecutionError::new("Codex returned no models. Check `codex login status` and your account access.")); }
            Ok(ProviderCapabilities { provider: Provider::Codex, models })
        }).await.map_err(|_| ExecutionError::new("Codex model discovery timed out. Check your CLI installation and sign-in."))?
    }

    async fn run_turn(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        mut responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        let mut command = Command::new(&self.executable);
        command
            .args(["app-server", "--stdio"])
            .current_dir(&request.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            ExecutionError::with_source(
                format!(
                    "could not start Codex at {}: {error}. Set SMALLER_TALK_CODEX_BIN if it is not on PATH",
                    self.executable.display()
                ),
                error,
            )
        })?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExecutionError::new("Codex app-server did not open stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ExecutionError::new("Codex app-server did not open stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ExecutionError::new("Codex app-server did not open stderr"))?;
        let stderr_task = tokio::spawn(read_stderr(stderr));
        let mut stdout = BufReader::new(stdout);

        send_request(
            &mut stdin,
            1,
            "initialize",
            json!({
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "title": CLIENT_TITLE,
                    "version": CLIENT_VERSION
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false
                }
            }),
        )
        .await?;
        let _ = read_response(&mut stdout, &mut stdin, 1).await?;
        send_notification(&mut stdin, "initialized", json!({})).await?;

        let thread_method = match request.binding.as_ref() {
            Some(binding) if binding.adapter_key == ADAPTER_KEY => "thread/resume",
            Some(binding) => {
                return Err(ExecutionError::new(format!(
                    "chat is bound to unsupported adapter {:?}",
                    binding.adapter_key
                )));
            }
            None => "thread/start",
        };
        let mut thread_params =
            json!({ "cwd": request.workspace.to_string_lossy(), "model": request.settings.model });
        if let Some(binding) = &request.binding {
            thread_params["threadId"] = json!(binding.native_session_id);
            thread_params["excludeTurns"] = json!(true);
        }
        send_request(&mut stdin, 2, thread_method, thread_params).await?;
        let thread_response = read_response(&mut stdout, &mut stdin, 2).await?;
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ExecutionError::new("Codex app-server returned a thread response without an ID")
            })?
            .to_owned();
        let binding = ExecutionBinding {
            adapter_key: ADAPTER_KEY.to_owned(),
            native_session_id: thread_id.clone(),
        };
        events
            .send(ExecutionEvent::BindingEstablished(binding))
            .map_err(|error| {
                ExecutionError::with_source(
                    format!("conversation stopped before Codex could continue: {error}"),
                    error,
                )
            })?;

        let mut input =
            vec![json!({ "type": "text", "text": request.full_prompt(), "text_elements": [] })];
        for file in request.attachments.iter().filter(|a| a.is_image()) {
            input.push(json!({ "type": "localImage", "path": file.path }));
        }
        let mut params = json!({
            "threadId": thread_id, "cwd": request.workspace,
            "input": input, "model": request.settings.model, "effort": request.settings.effort,
        });
        // Explicitly set both modes, so returning from Plan does not inherit Plan instructions.
        // A collaboration preset requires a concrete model. The application resolves defaults.
        if let Some(model) = &request.settings.model {
            params["collaborationMode"] = json!({
                "mode": if request.settings.plan_mode { "plan" } else { "default" },
                "settings": { "model": model, "reasoning_effort": request.settings.effort, "developer_instructions": null }
            });
        } else if request.settings.plan_mode {
            return Err(ExecutionError::new(
                "Choose an available model before entering Plan mode",
            ));
        }
        send_request(&mut stdin, 3, "turn/start", params).await?;
        let _ = events.send(ExecutionEvent::Activity(Activity::Working));

        let mut text_items: Vec<(String, String)> = Vec::new();
        let mut turn_response_received = false;
        let mut turn_completed = false;
        while !turn_completed {
            let value = read_message(&mut stdout).await?;
            if value.get("method").is_some() && value.get("id").is_some() {
                handle_interaction(&mut stdin, &value, &events, &mut responses).await?;
                continue;
            }
            if value.get("id") == Some(&json!(3)) && value.get("error").is_some() {
                return Err(rpc_error(&value));
            }
            if value.get("id") == Some(&json!(3)) {
                turn_response_received = true;
                continue;
            }

            match value.get("method").and_then(Value::as_str) {
                Some("item/agentMessage/delta" | "item/plan/delta") => {
                    let params = &value["params"];
                    if params["threadId"].as_str() == Some(thread_id.as_str())
                        && let Some(delta) = params["delta"].as_str()
                    {
                        let _ = events.send(ExecutionEvent::Activity(Activity::Responding));
                        events
                            .send(ExecutionEvent::TextDelta(delta.to_owned()))
                            .map_err(|error| {
                                ExecutionError::with_source(
                                    format!(
                                        "conversation stopped while Codex was responding: {error}"
                                    ),
                                    error,
                                )
                            })?;
                    }
                }
                Some("item/completed") => {
                    let item = &value["params"]["item"];
                    if matches!(item["type"].as_str(), Some("plan" | "agentMessage"))
                        && let Some(text) = item["text"].as_str()
                    {
                        let id = item["id"].as_str().unwrap_or("answer").to_owned();
                        if let Some((_, old)) = text_items.iter_mut().find(|(key, _)| *key == id) {
                            if *old != text {
                                *old = text.into();
                                let full = text_items
                                    .iter()
                                    .map(|(_, text)| text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n\n");
                                let _ = events.send(ExecutionEvent::TextSnapshot(full));
                            }
                        } else {
                            // Completion-only messages (and the final proposed plan) must not disappear.
                            text_items.push((id, text.into()));
                            let full = text_items
                                .iter()
                                .map(|(_, text)| text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            let _ = events.send(ExecutionEvent::TextSnapshot(full));
                        }
                    }
                }
                Some("item/reasoning/summaryTextDelta" | "item/reasoning/textDelta") => {
                    let _ = events.send(ExecutionEvent::Activity(Activity::Thinking));
                }
                Some("item/started") => {
                    let activity = match value.pointer("/params/item/type").and_then(Value::as_str)
                    {
                        Some("reasoning") => Activity::Thinking,
                        Some("agentMessage" | "plan") => Activity::Responding,
                        _ => Activity::Working,
                    };
                    let _ = events.send(ExecutionEvent::Activity(activity));
                }
                Some("turn/completed") => {
                    let params = &value["params"];
                    if params["threadId"].as_str() != Some(thread_id.as_str()) {
                        continue;
                    }
                    turn_completed = true;
                    match params.pointer("/turn/status").and_then(Value::as_str) {
                        Some("completed") => {}
                        Some(status) => {
                            let detail = params
                                .pointer("/turn/error/message")
                                .and_then(Value::as_str)
                                .map(|message| format!(" ({message})"))
                                .unwrap_or_default();
                            return Err(ExecutionError::new(format!(
                                "Codex turn ended with status {status}{detail}"
                            )));
                        }
                        None => {
                            return Err(ExecutionError::new(
                                "Codex completed a turn without reporting its status",
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        // The app-server exits after stdin closes. Bound shutdown in case a future version keeps
        // ancillary work alive after its turn has completed.
        drop(stdin);
        match timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) if status.success() => {}
            Ok(Ok(status)) => {
                let stderr = match stderr_task.await {
                    Ok(stderr) => stderr,
                    Err(error) => {
                        return Err(ExecutionError::with_source(
                            format!("could not read Codex stderr: {error}"),
                            error,
                        ));
                    }
                };
                return Err(ExecutionError::new(format!(
                    "Codex app-server exited with {status}: {}",
                    summarize_stderr(&stderr)
                )));
            }
            Ok(Err(error)) => {
                return Err(ExecutionError::with_source(
                    format!("could not wait for Codex app-server: {error}"),
                    error,
                ));
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        if !turn_response_received {
            return Err(ExecutionError::new(
                "Codex completed a turn without acknowledging turn/start",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionAdapter for CodexAdapter {
    async fn capabilities(
        &self,
        _provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        self.discover_models().await
    }
    async fn execute_interactive(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        self.run_turn(request, events, responses).await
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        let (_, responses) = mpsc::unbounded_channel();
        self.run_turn(request, events, responses).await
    }
}

async fn send_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), ExecutionError> {
    write_message(
        stdin,
        &json!({ "id": id, "method": method, "params": params }),
    )
    .await
}

async fn send_notification(
    stdin: &mut ChildStdin,
    method: &str,
    params: Value,
) -> Result<(), ExecutionError> {
    write_message(stdin, &json!({ "method": method, "params": params })).await
}

async fn write_message(stdin: &mut ChildStdin, message: &Value) -> Result<(), ExecutionError> {
    let mut bytes = serde_json::to_vec(message).map_err(|error| {
        ExecutionError::with_source(format!("could not encode Codex request: {error}"), error)
    })?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await.map_err(|error| {
        ExecutionError::with_source(format!("could not write to Codex: {error}"), error)
    })?;
    stdin.flush().await.map_err(|error| {
        ExecutionError::with_source(format!("could not flush Codex request: {error}"), error)
    })
}

async fn read_response(
    stdout: &mut BufReader<ChildStdout>,
    stdin: &mut ChildStdin,
    id: u64,
) -> Result<Value, ExecutionError> {
    loop {
        let value = read_message(stdout).await?;
        if value.get("method").is_some() && value.get("id").is_some() {
            handle_server_request(stdin, &value).await?;
            continue;
        }
        if value.get("id") != Some(&json!(id)) {
            continue;
        }
        if value.get("error").is_some() {
            return Err(rpc_error(&value));
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

async fn read_message(stdout: &mut BufReader<ChildStdout>) -> Result<Value, ExecutionError> {
    let mut line = String::new();
    let bytes_read = stdout.read_line(&mut line).await.map_err(|error| {
        ExecutionError::with_source(format!("could not read Codex output: {error}"), error)
    })?;
    if bytes_read == 0 {
        return Err(ExecutionError::new(
            "Codex app-server closed its output before the turn completed",
        ));
    }
    serde_json::from_str(&line).map_err(|error| {
        ExecutionError::with_source(format!("Codex sent invalid JSON: {error}"), error)
    })
}

async fn handle_server_request(
    stdin: &mut ChildStdin,
    request: &Value,
) -> Result<(), ExecutionError> {
    write_message(stdin, &json!({ "id": request["id"], "error": { "code": -32601, "message": "This interaction is unavailable before a turn starts" } })).await?;
    Err(ExecutionError::new(format!(
        "Unsupported Codex interaction during startup: {}",
        request["method"]
    )))
}

async fn handle_interaction(
    stdin: &mut ChildStdin,
    request: &Value,
    events: &mpsc::UnboundedSender<ExecutionEvent>,
    responses: &mut mpsc::UnboundedReceiver<InteractionResponse>,
) -> Result<(), ExecutionError> {
    let method = request["method"].as_str().unwrap_or_default();
    let params = &request["params"];
    let id = request["id"].to_string();
    let kind = match method {
        "item/tool/requestUserInput" => {
            InteractionKind::Questions(parse_questions(&params["questions"]))
        }
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval"
        | "applyPatchApproval"
        | "execCommandApproval" => InteractionKind::Approval {
            detail: describe_approval(params),
        },
        _ => {
            write_message(stdin, &json!({ "id": request["id"], "error": { "code": -32601, "message": "Smaller Talk does not support this interaction" } })).await?;
            return Err(ExecutionError::new(format!(
                "Unsupported Codex interaction: {method}"
            )));
        }
    };
    if matches!(&kind, InteractionKind::Questions(questions) if questions.is_empty()) {
        return Err(ExecutionError::new("Codex sent an empty question request"));
    }
    let _ = events.send(ExecutionEvent::Activity(Activity::Waiting));
    events
        .send(ExecutionEvent::Interaction(Interaction {
            id: id.clone(),
            title: if matches!(kind, InteractionKind::Questions(_)) {
                "A question from Codex"
            } else {
                "Codex needs approval"
            }
            .into(),
            kind,
        }))
        .map_err(|_| {
            ExecutionError::new("The chat closed before the interaction could be shown")
        })?;
    let answer = loop {
        let answer = responses.recv().await.ok_or_else(|| {
            ExecutionError::new("The chat closed while Codex was waiting for an answer")
        })?;
        if answer.id == id {
            break answer;
        }
    };
    let result = match method {
        "item/tool/requestUserInput" => {
            let answers: serde_json::Map<String, Value> = answer
                .answers
                .into_iter()
                .map(|(id, answers)| (id, json!({ "answers": answers })))
                .collect();
            json!({ "answers": answers })
        }
        "item/permissions/requestApproval" => {
            json!({ "permissions": if answer.approved { params["permissions"].clone() } else { json!({}) }, "scope": "turn" })
        }
        "applyPatchApproval" | "execCommandApproval" => {
            json!({ "decision": if answer.approved { "approved" } else { "denied" } })
        }
        _ => json!({ "decision": if answer.approved { "accept" } else { "decline" } }),
    };
    write_message(stdin, &json!({ "id": request["id"], "result": result })).await?;
    let _ = events.send(ExecutionEvent::Activity(Activity::Working));
    Ok(())
}

fn describe_approval(params: &Value) -> String {
    let mut lines = Vec::new();
    for (key, label) in [
        ("command", "Command"),
        ("cwd", "Working folder"),
        ("reason", "Reason"),
        ("grantRoot", "Requested folder"),
        ("permissions", "Requested permissions"),
        ("networkApprovalContext", "Network access"),
    ] {
        if let Some(value) = params.get(key).filter(|v| !v.is_null()) {
            let value = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default());
            lines.push(format!("{label}:\n{value}"));
        }
    }
    if lines.is_empty() {
        "Allow the agent’s requested action for this turn?".into()
    } else {
        lines.join("\n\n")
    }
}

pub(crate) fn parse_questions(value: &Value) -> Vec<Question> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, q)| Question {
            id: q["id"]
                .as_str()
                .or_else(|| q["question"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| index.to_string()),
            prompt: q["question"]
                .as_str()
                .or_else(|| q["prompt"].as_str())
                .unwrap_or("Your response")
                .into(),
            options: q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    o["label"]
                        .as_str()
                        .or_else(|| o.as_str())
                        .map(str::to_owned)
                })
                .collect(),
            multiple: q["multiSelect"].as_bool().unwrap_or(false),
        })
        .collect()
}

fn rpc_error(value: &Value) -> ExecutionError {
    let error = &value["error"];
    let message = error["message"]
        .as_str()
        .or_else(|| error.as_str())
        .unwrap_or("Codex app-server returned an unspecified error");
    ExecutionError::new(message.to_owned())
}

const STDERR_LIMIT: usize = 16 * 1024;

pub(crate) async fn read_stderr(mut stderr: impl tokio::io::AsyncRead + Unpin) -> String {
    let mut retained = Vec::with_capacity(STDERR_LIMIT);
    let mut buffer = [0; 4096];
    // Keep the pipe open and drain to EOF even after the diagnostic prefix is full.
    while let Ok(count) = stderr.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        let keep = count.min(STDERR_LIMIT - retained.len());
        retained.extend_from_slice(&buffer[..keep]);
    }
    // Diagnostics can contain invalid UTF-8, or the limit can split a character.
    let mut output = String::from_utf8_lossy(&retained).into_owned();
    let mut end = output.len().min(STDERR_LIMIT);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    output
}

fn summarize_stderr(stderr: &str) -> &str {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        "no diagnostic output"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use smaller_talk_application::{ExecutionEvent, ExecutionRequest};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use super::CodexAdapter;
    use smaller_talk_application::ExecutionAdapter;

    fn request(workspace: PathBuf) -> ExecutionRequest {
        ExecutionRequest {
            chat_id: Uuid::new_v4(),
            binding: None,
            workspace,
            prompt: "continue".into(),
            settings: Default::default(),
            attachments: vec![],
            prior_context: None,
        }
    }

    #[tokio::test]
    async fn stderr_is_drained_past_the_retention_limit_without_breaking_the_child() {
        let mut child = tokio::process::Command::new("python3")
            .args([
                "-c",
                "import os; os.write(2, b'x' * (1024 * 1024)); print('finished')",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start noisy subprocess");
        let stderr = child.stderr.take().expect("stderr pipe");
        let reader = tokio::spawn(super::read_stderr(stderr));
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
                .await
                .expect("child must not block on stderr")
                .expect("child output");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"finished\n");
        let diagnostics = reader.await.expect("stderr reader");
        assert_eq!(diagnostics.len(), super::STDERR_LIMIT);
        assert!(diagnostics.bytes().all(|byte| byte == b'x'));
    }

    #[tokio::test]
    async fn stderr_retention_tolerates_invalid_and_split_utf8() {
        let bytes = vec![0xff; super::STDERR_LIMIT + 10];
        let diagnostics = super::read_stderr(&bytes[..]).await;
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.len() <= super::STDERR_LIMIT);
        let mut bytes = vec![b'x'; super::STDERR_LIMIT - 1];
        bytes.extend_from_slice("é".as_bytes());
        let diagnostics = super::read_stderr(&bytes[..]).await;
        assert!(diagnostics.len() <= super::STDERR_LIMIT);
    }

    #[tokio::test]
    async fn maps_app_server_deltas_and_resumes_the_stored_thread() {
        let directory = tempfile::tempdir().expect("fixture directory");
        let script = directory.path().join("fake-codex.py");
        std::fs::write(
            &script,
            r##"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        print(json.dumps({"id": request["id"], "result": {"protocolVersion": "current"}}), flush=True)
    elif method == "initialized":
        pass
    elif method == "thread/resume":
        assert request["params"]["threadId"] == "native-thread-123"
        print(json.dumps({"id": request["id"], "result": {"thread": {"id": "native-thread-123"}}}), flush=True)
    elif method == "turn/start":
        print(json.dumps({"id": request["id"], "result": {"turn": {"id": "turn-1"}}}), flush=True)
        print(json.dumps({"method": "item/agentMessage/delta", "params": {"threadId": "native-thread-123", "delta": "streamed answer"}}), flush=True)
        print(json.dumps({"method": "turn/completed", "params": {"threadId": "native-thread-123", "turn": {"status": "completed"}}}), flush=True)
"##,
        )
        .expect("write fixture");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");

        let (sender, mut receiver) = mpsc::unbounded_channel();
        CodexAdapter::new(&script)
            .execute(
                ExecutionRequest {
                    chat_id: Uuid::new_v4(),
                    binding: Some(smaller_talk_domain::ExecutionBinding {
                        adapter_key: "codex".into(),
                        native_session_id: "native-thread-123".into(),
                    }),
                    workspace: directory.path().to_path_buf(),
                    prompt: "continue".into(),
                    settings: Default::default(),
                    attachments: vec![],
                    prior_context: None,
                },
                sender,
            )
            .await
            .expect("fixture turn completes");

        assert!(matches!(
            receiver.recv().await,
            Some(ExecutionEvent::BindingEstablished(binding))
                if binding.native_session_id == "native-thread-123"
        ));
        let mut text = String::new();
        let mut working = false;
        while let Some(event) = receiver.recv().await {
            match event {
                ExecutionEvent::TextDelta(delta) => text.push_str(&delta),
                ExecutionEvent::Activity(_) => working = true,
                _ => {}
            }
        }
        assert_eq!(text, "streamed answer");
        assert!(working);
    }

    #[tokio::test]
    async fn spawn_failure_keeps_its_opaque_io_source() {
        let directory = tempfile::tempdir().expect("fixture directory");
        let missing = directory.path().join("missing-codex");
        let (sender, _receiver) = mpsc::unbounded_channel();
        let error = CodexAdapter::new(missing)
            .execute(request(directory.path().to_path_buf()), sender)
            .await
            .expect_err("missing executable fails to start");

        assert!(error.to_string().contains("could not start Codex"));
        assert!(
            Error::source(&error)
                .expect("spawn diagnostic is retained")
                .is::<std::io::Error>()
        );
    }

    #[tokio::test]
    async fn malformed_app_server_json_keeps_its_serde_source() {
        let directory = tempfile::tempdir().expect("fixture directory");
        let script = directory.path().join("invalid-codex.py");
        std::fs::write(
            &script,
            "#!/usr/bin/env python3\nprint('{invalid-json', flush=True)\n",
        )
        .expect("write fixture");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make fixture executable");

        let (sender, _receiver) = mpsc::unbounded_channel();
        let error = CodexAdapter::new(&script)
            .execute(request(directory.path().to_path_buf()), sender)
            .await
            .expect_err("invalid server output fails to parse");

        assert!(error.to_string().contains("Codex sent invalid JSON"));
        assert!(
            Error::source(&error)
                .expect("parse diagnostic is retained")
                .is::<serde_json::Error>()
        );
    }
}
