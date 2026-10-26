use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use serde_json::{Value, json};
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest, SteerCommand, SteerOutcome,
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

struct CodexPermissions {
    approval: &'static str,
    reviewer: &'static str,
    sandbox: &'static str,
    policy: &'static str,
}

fn codex_permissions(mode: PermissionMode) -> Option<CodexPermissions> {
    let (approval, reviewer, sandbox, policy) = match mode {
        PermissionMode::ProviderDefault => return None,
        PermissionMode::Supervised => ("untrusted", "user", "read-only", "readOnly"),
        PermissionMode::AcceptEdits => ("on-request", "user", "workspace-write", "workspaceWrite"),
        PermissionMode::Auto => (
            "on-request",
            "auto_review",
            "workspace-write",
            "workspaceWrite",
        ),
        PermissionMode::FullAccess => ("never", "user", "danger-full-access", "dangerFullAccess"),
    };
    Some(CodexPermissions {
        approval,
        reviewer,
        sandbox,
        policy,
    })
}

#[derive(Clone, Debug)]
pub struct CodexAdapter {
    executable: PathBuf,
}

impl CodexAdapter {
    pub(crate) fn executable(&self) -> &std::path::Path {
        &self.executable
    }
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub(crate) async fn discover_skills(
        &self,
        workspace: &std::path::Path,
    ) -> Result<Vec<SkillDescriptor>, ExecutionError> {
        timeout(Duration::from_secs(25), async {
            let mut child = Command::new(&self.executable).args(["app-server", "--stdio"]).current_dir(workspace)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()
                .map_err(|e| ExecutionError::with_source("Could not start Codex skill discovery", e))?;
            let mut stdin = child.stdin.take().ok_or_else(|| ExecutionError::new("Codex stdin is unavailable"))?;
            let mut stdout = BufReader::new(child.stdout.take().ok_or_else(|| ExecutionError::new("Codex stdout is unavailable"))?);
            if let Some(stderr) = child.stderr.take() { tokio::spawn(read_stderr(stderr)); }
            send_request(&mut stdin, 10, "initialize", json!({"clientInfo":{"name":CLIENT_NAME,"version":CLIENT_VERSION},"capabilities":{"experimentalApi":true}})).await?;
            read_response(&mut stdout, &mut stdin, 10).await?;
            send_notification(&mut stdin, "initialized", json!({})).await?;
            send_request(&mut stdin, 11, "skills/list", json!({"cwds":[workspace],"forceReload":true})).await?;
            let result = read_response(&mut stdout, &mut stdin, 11).await?;
            let scoped = result["data"].as_array().and_then(|rows| rows.iter().find(|r| r["cwd"].as_str().is_some_and(|p| std::path::Path::new(p) == workspace)))
                .ok_or_else(|| ExecutionError::new("Codex did not return skills for this folder"))?;
            if scoped["errors"].as_array().is_some_and(|e| !e.is_empty()) { return Err(ExecutionError::new(format!("Codex skill discovery reported: {}", scoped["errors"]))); }
            let mut entries = Vec::new();
            for skill in scoped["skills"].as_array().into_iter().flatten() {
                let name = skill["name"].as_str().ok_or_else(|| ExecutionError::new("Codex skill has no name"))?;
                let path = skill["path"].as_str().ok_or_else(|| ExecutionError::new("Codex skill has no source path"))?;
                entries.push(SkillDescriptor {
                    reference: SkillReference { provider: Provider::Codex, name: name.into(), path: path.into(), workspace: workspace.to_owned() },
                    description: skill["interface"]["shortDescription"].as_str().or_else(|| skill["description"].as_str()).unwrap_or_default().into(),
                    source: skill["scope"].as_str().unwrap_or("Provider").into(), command: name.into(),
                    unavailable: (skill["enabled"].as_bool() == Some(false)).then(|| "This skill is disabled in Codex".into()),
                });
            }
            drop(stdin);
            if timeout(Duration::from_secs(2), child.wait()).await.is_err() { let _ = child.kill().await; let _ = child.wait().await; }
            Ok(entries)
        }).await.map_err(|_| ExecutionError::new("Codex skill discovery timed out"))?
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
        mut controls: mpsc::UnboundedReceiver<SteerCommand>,
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
        let permissions = codex_permissions(request.settings.permission_mode());
        if let Some(permissions) = &permissions {
            thread_params["approvalPolicy"] = json!(permissions.approval);
            thread_params["approvalsReviewer"] = json!(permissions.reviewer);
            thread_params["sandbox"] = json!(permissions.sandbox);
        }
        if !request.instructions.text.is_empty() {
            thread_params["developerInstructions"] = json!(request.instructions.text);
        }
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
            instructions_fingerprint: None,
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

        let mut text = request.full_prompt();
        let mut skills = request.skills.clone();
        for replay in &request.retry_inputs {
            let header = format!(
                "\n\nConfirmed user steering accepted for this interrupted turn (message ID {}):\n",
                replay.id
            );
            let start = text.len() + header.len();
            text.push_str(&header);
            text.push_str(&replay.text_with_files());
            skills.extend(replay.skills.iter().cloned().map(|mut m| {
                m.range.start += start;
                m.range.end += start;
                m
            }));
        }
        // Original ranges refer to the current prompt, before full_prompt's history prefix.
        let prefix = request
            .prior_context
            .as_ref()
            .map_or(0, |c| c.len() + "\n\nCurrent user message:\n".len());
        for mention in skills.iter_mut().take(request.skills.len()) {
            mention.range.start += prefix;
            mention.range.end += prefix;
        }
        let input = native_input(&text, &request.attachments, &skills)?;
        let mut params = json!({
            "threadId": thread_id, "cwd": request.workspace,
            "input": input, "model": request.settings.model, "effort": request.settings.effort,
        });
        if let Some(permissions) = &permissions {
            params["approvalPolicy"] = json!(permissions.approval);
            params["approvalsReviewer"] = json!(permissions.reviewer);
            params["sandboxPolicy"] = json!({"type": permissions.policy});
        }
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
        let mut offsets = std::collections::HashMap::<String, usize>::new();
        let mut native_turn: Option<String> = None;
        let mut pending_steer: Option<SteerCommand> = None;
        let mut controls_open = true;
        // A dedicated reader avoids cancelling read_line midway through a JSON frame.
        let (frames, mut incoming) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let frame = read_message(&mut stdout).await;
                let failed = frame.is_err();
                if frames.send(frame).await.is_err() || failed {
                    break;
                }
            }
        });
        let mut turn_response_received = false;
        let mut turn_completed = false;
        let mut final_text = None;
        while !turn_completed {
            let value = tokio::select! {
                frame = incoming.recv() => frame.ok_or_else(|| ExecutionError::new("Codex disconnected"))??,
                command = controls.recv(), if controls_open && pending_steer.is_none() => {
                    let Some(command) = command else { controls_open = false; continue; };
                    let Some(turn_id) = native_turn.as_ref() else { let _ = command.result.send(SteerOutcome::NotRunning); continue; };
                    if let Err(error) = crate::steering::validate(&command.input).await {
                        let _ = command.result.send(SteerOutcome::Rejected(error.to_string())); continue;
                    }
                    if !command.input.skills.is_empty() {
                        let entries = match self.discover_skills(&request.workspace).await {
                            Ok(entries) => entries,
                            Err(error) => {
                                let _ = command.result.send(SteerOutcome::Rejected(error.to_string()));
                                continue;
                            }
                        };
                        if let Err(error) = crate::skills::validate_selected(&entries, Provider::Codex, &request.workspace, &command.input.prompt, &command.input.skills) {
                            let _ = command.result.send(SteerOutcome::Rejected(error.to_string())); continue;
                        }
                    }
                    let input = native_input(&command.input.text_with_files(), &command.input.attachments, &command.input.skills)?;
                    send_request(&mut stdin, 4, "turn/steer", json!({"threadId": thread_id, "expectedTurnId": turn_id, "input":input})).await?;
                    pending_steer = Some(command);
                    continue;
                }
            };
            if value["method"] == "skills/changed" {
                let _ = events.send(ExecutionEvent::SkillsChanged);
            }
            if value.get("id") == Some(&json!(4)) {
                if let Some(command) = pending_steer.take() {
                    if value.get("error").is_some() {
                        let _ = command
                            .result
                            .send(SteerOutcome::Rejected(rpc_error(&value).to_string()));
                    } else if value.pointer("/result/turnId").and_then(Value::as_str)
                        == native_turn.as_deref()
                    {
                        for (id, text) in &text_items {
                            *offsets.entry(id.clone()).or_default() += text.len();
                        }
                        text_items.clear();
                        let _ = events.send(ExecutionEvent::InputAccepted(command.input));
                        let _ = command.result.send(SteerOutcome::Accepted);
                    } else {
                        let _ = command.result.send(SteerOutcome::Uncertain(
                            "Codex did not acknowledge the expected turn.".into(),
                        ));
                    }
                }
                continue;
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                handle_interaction(&mut stdin, &value, &events, &mut responses).await?;
                continue;
            }
            if value.get("id") == Some(&json!(3)) && value.get("error").is_some() {
                return Err(rpc_error(&value));
            }
            if value.get("id") == Some(&json!(3)) {
                turn_response_received = true;
                native_turn = value
                    .pointer("/result/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(native_turn);
                let _ = events.send(ExecutionEvent::SteeringAvailable(native_turn.is_some()));
                continue;
            }

            match value.get("method").and_then(Value::as_str) {
                Some("turn/started") => {
                    native_turn = value
                        .pointer("/params/turn/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(native_turn);
                    let _ = events.send(ExecutionEvent::SteeringAvailable(native_turn.is_some()));
                }
                Some("item/agentMessage/delta" | "item/plan/delta") => {
                    let params = &value["params"];
                    if params["threadId"].as_str() == Some(thread_id.as_str())
                        && let Some(delta) = params["delta"].as_str()
                    {
                        let item_id = params["itemId"].as_str().unwrap_or("answer").to_owned();
                        let is_new_item = !text_items.iter().any(|(key, _)| *key == item_id);
                        let has_prior_text = text_items
                            .iter()
                            .any(|(key, text)| key != &item_id && !text.is_empty());
                        if let Some((_, text)) =
                            text_items.iter_mut().find(|(key, _)| *key == item_id)
                        {
                            text.push_str(delta);
                        } else {
                            text_items.push((item_id, delta.to_owned()));
                        }
                        let _ = events.send(ExecutionEvent::Activity(Activity::Responding));
                        let event = if is_new_item && has_prior_text {
                            ExecutionEvent::TextSnapshot(
                                text_items
                                    .iter()
                                    .map(|(_, text)| text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n\n"),
                            )
                        } else {
                            ExecutionEvent::TextDelta(delta.to_owned())
                        };
                        events.send(event).map_err(|error| {
                            ExecutionError::with_source(
                                format!("conversation stopped while Codex was responding: {error}"),
                                error,
                            )
                        })?;
                    }
                }
                Some("item/completed") => {
                    let item = &value["params"]["item"];
                    if item["type"] == "agentMessage"
                        && item["phase"].as_str().is_none_or(|p| p == "final_answer")
                    {
                        final_text = item["text"].as_str().map(str::to_owned);
                    } else {
                        final_text = None;
                    }
                    if matches!(item["type"].as_str(), Some("plan" | "agentMessage"))
                        && let Some(text) = item["text"].as_str()
                    {
                        let id = item["id"].as_str().unwrap_or("answer").to_owned();
                        let text = text
                            .get(offsets.get(&id).copied().unwrap_or(0)..)
                            .unwrap_or("");
                        if let Some((_, old)) = text_items.iter_mut().find(|(key, _)| *key == id) {
                            *old = text.into();
                        } else {
                            text_items.push((id, text.into()));
                        }
                        // Completion is the authoritative item boundary. Publish the joined
                        // snapshot even when the completed text equals its streamed deltas.
                        let full = text_items
                            .iter()
                            .map(|(_, text)| text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        let _ = events.send(ExecutionEvent::TextSnapshot(full));
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

        if let Some(command) = pending_steer {
            let _ = command.result.send(SteerOutcome::Uncertain(
                "Codex finished before confirming steering. Review the conversation.".into(),
            ));
        }
        while let Ok(command) = controls.try_recv() {
            let _ = command.result.send(SteerOutcome::NotRunning);
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
        if !request.settings.plan_mode
            && let Some(text) = final_text
        {
            let _ = events.send(ExecutionEvent::FinalAnswerText(text));
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
    async fn compact(
        &self,
        request: smaller_talk_application::CompactionRequest,
    ) -> Result<smaller_talk_application::CompactionOutcome, ExecutionError> {
        let (mut child, mut stdin, mut stdout) = timeout(Duration::from_secs(25), async {
            let mut child = Command::new(&self.executable).args(["app-server", "--stdio"])
                .current_dir(&request.workspace).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true)
                .spawn().map_err(|e| ExecutionError::with_source("Could not start Codex compaction", e))?;
            let mut stdin = child.stdin.take().ok_or_else(|| ExecutionError::new("Codex stdin unavailable"))?;
            let mut stdout = BufReader::new(child.stdout.take().ok_or_else(|| ExecutionError::new("Codex stdout unavailable"))?);
            send_request(&mut stdin, 1, "initialize", json!({"clientInfo":{"name":CLIENT_NAME,"version":CLIENT_VERSION},"capabilities":{"experimentalApi":true}})).await?;
            read_response(&mut stdout, &mut stdin, 1).await?;
            send_notification(&mut stdin, "initialized", json!({})).await?;
            let mut params = json!({"threadId":request.binding.native_session_id,"cwd":request.workspace,"excludeTurns":true,"model":request.settings.model});
            if !request.instructions.text.is_empty() { params["developerInstructions"] = json!(request.instructions.text); }
            send_request(&mut stdin, 2, "thread/resume", params).await?;
            read_response(&mut stdout, &mut stdin, 2).await?;
            Ok::<_, ExecutionError>((child, stdin, stdout))
        }).await.map_err(|_| ExecutionError::new("Codex startup timed out before compaction."))??;
        let result = timeout(Duration::from_secs(180), async {
            send_request(&mut stdin, 3, "thread/compact/start", json!({"threadId":request.binding.native_session_id})).await?;
            let mut accepted = false;
            let mut compacted = false;
            let mut completed = false;
            loop {
                let value = read_message(&mut stdout).await?;
                if value["id"] == 3 {
                    if !value["error"].is_null() { return Err(ExecutionError::new(format!("Codex could not compact this session: {}", value["error"]))); }
                    accepted = true;
                }
                if value["params"]["threadId"] == request.binding.native_session_id {
                    if value["method"] == "item/completed" && value["params"]["item"]["type"] == "contextCompaction" { compacted = true; }
                    if value["method"] == "turn/completed" {
                        if value["params"]["turn"]["status"] != "completed" { return Err(ExecutionError::new(format!("Codex compaction did not complete: {}", value["params"]["turn"]))); }
                        completed = true;
                    }
                    if value["method"] == "error" { return Err(ExecutionError::new(format!("Codex compaction failed: {}", value["params"]))); }
                }
                if accepted && completed {
                    return if compacted { Ok(smaller_talk_application::CompactionOutcome::Compacted) } else { Err(ExecutionError::new("Codex finished without confirming context compaction.")) };
                }
            }
        }).await.unwrap_or_else(|_| Err(ExecutionError::new("Codex compaction timed out; its outcome is uncertain. Review the session before trying again.")));
        let _ = child.kill().await;
        let _ = child.wait().await;
        result
    }

    async fn capabilities(
        &self,
        _provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        self.discover_models().await
    }
    async fn execute_controlled(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
        controls: mpsc::UnboundedReceiver<SteerCommand>,
    ) -> Result<(), ExecutionError> {
        self.run_turn(request, events, responses, controls).await
    }
    async fn execute_interactive(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        let (_, controls) = mpsc::unbounded_channel();
        self.run_turn(request, events, responses, controls).await
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        let (_, responses) = mpsc::unbounded_channel();
        let (_, controls) = mpsc::unbounded_channel();
        self.run_turn(request, events, responses, controls).await
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
        | "execCommandApproval" => InteractionKind::Approval(crate::approvals::present(
            method,
            params,
            method == "item/permissions/requestApproval",
        )),
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

fn native_input(
    text: &str,
    attachments: &[Attachment],
    skills: &[SkillMention],
) -> Result<Vec<Value>, ExecutionError> {
    let mut input = vec![json!({"type":"text","text":text,"text_elements":[]})];
    let mut seen = std::collections::HashSet::new();
    for mention in skills {
        if !mention.matches(text) || mention.reference.provider != Provider::Codex {
            return Err(ExecutionError::new(
                "The selected skill no longer matches the message",
            ));
        }
        if seen.insert(&mention.reference.path) {
            input.push(
                json!({"type":"skill","name":mention.reference.name,"path":mention.reference.path}),
            );
        }
    }
    for file in attachments.iter().filter(|a| a.is_image()) {
        input.push(json!({"type":"localImage","path":file.path}));
    }
    Ok(input)
}

#[cfg(test)]
mod tests {
    use smaller_talk_domain::{Provider, SkillMention, SkillReference};
    #[test]
    fn skill_inputs_keep_distinct_sources_and_deduplicate_repeated_mentions() {
        let mention = |range, path: &str| SkillMention {
            range,
            reference: SkillReference {
                provider: Provider::Codex,
                name: "review".into(),
                path: path.into(),
                workspace: "/work".into(),
            },
        };
        let skills = vec![
            mention(0..7, "/one/SKILL.md"),
            mention(8..15, "/two/SKILL.md"),
            mention(16..23, "/one/SKILL.md"),
        ];
        let input = super::native_input("$review $review $review", &[], &skills).unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[1]["path"], "/one/SKILL.md");
        assert_eq!(input[2]["path"], "/two/SKILL.md");
        assert!(super::native_input("changed", &[], &skills).is_err());
    }

    use std::{error::Error, path::PathBuf};

    use smaller_talk_application::{ExecutionEvent, ExecutionRequest};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use super::CodexAdapter;
    use smaller_talk_application::ExecutionAdapter;

    fn request(workspace: PathBuf) -> ExecutionRequest {
        ExecutionRequest {
            skills: Vec::new(),
            retry_inputs: Vec::new(),
            instructions: Default::default(),
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
                    skills: Vec::new(),
                    retry_inputs: Vec::new(),
                    instructions: Default::default(),
                    chat_id: Uuid::new_v4(),
                    binding: Some(smaller_talk_domain::ExecutionBinding {
                        instructions_fingerprint: None,
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
