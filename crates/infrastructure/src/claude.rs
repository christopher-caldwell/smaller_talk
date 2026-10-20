use std::{collections::HashSet, path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest, SteerCommand, SteerOutcome,
};
use smaller_talk_domain::*;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::mpsc,
    time::timeout,
};

#[derive(Clone)]
pub struct ClaudeAdapter {
    executable: PathBuf,
}
impl ClaudeAdapter {
    pub(crate) fn executable(&self) -> &std::path::Path {
        &self.executable
    }
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    async fn start(
        &self,
        request: Option<&ExecutionRequest>,
    ) -> Result<(Child, ChildStdin, BufReader<ChildStdout>), ExecutionError> {
        let mut cmd = Command::new(&self.executable);
        cmd.args([
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--replay-user-messages",
            "--permission-prompt-tool",
            "stdio",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        if let Some(request) = request {
            cmd.current_dir(&request.workspace);
            cmd.args([
                "--permission-mode",
                if request.settings.plan_mode {
                    "plan"
                } else {
                    "default"
                },
            ]);
            if let Some(model) = &request.settings.model {
                cmd.args(["--model", model]);
            }
            if let Some(effort) = &request.settings.effort {
                cmd.args(["--effort", effort]);
            }
            if let Some(binding) = &request.binding {
                cmd.args(["--resume", &binding.native_session_id]);
            }
            let mut directories = HashSet::new();
            for file in &request.attachments {
                if let Some(parent) = file.path.parent() {
                    directories.insert(parent.to_path_buf());
                }
            }
            for directory in directories {
                cmd.arg("--add-dir").arg(directory);
            }
        }
        let mut child = cmd.spawn().map_err(|e| ExecutionError::with_source("Could not start Claude Code. Install/sign in to Claude or set SMALLER_TALK_CLAUDE_BIN.", e))?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(crate::codex::read_stderr(stderr));
        }
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExecutionError::new("Claude stdin is unavailable"))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| ExecutionError::new("Claude stdout is unavailable"))?,
        );
        Ok((child, stdin, stdout))
    }

    async fn initialize(
        stdin: &mut ChildStdin,
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<Value, ExecutionError> {
        write(stdin, &json!({ "type": "control_request", "request_id": "initialize", "request": { "subtype": "initialize" } })).await?;
        timeout(Duration::from_secs(25), async {
            loop {
                let value = read(stdout).await?;
                if value["type"] == "control_response"
                    && value["response"]["request_id"] == "initialize"
                {
                    if value["response"]["subtype"] != "success" {
                        return Err(ExecutionError::new(format!(
                            "Claude initialization failed: {}",
                            value["response"]["error"]
                        )));
                    }
                    return Ok(value["response"]["response"].clone());
                }
                if value["type"] == "control_request" {
                    return Err(ExecutionError::new(
                        "Claude requested an unsupported interaction during startup",
                    ));
                }
            }
        })
        .await
        .map_err(|_| {
            ExecutionError::new(
                "Claude startup timed out. Check your CLI installation and sign-in.",
            )
        })?
    }

    async fn run(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        mut responses: mpsc::UnboundedReceiver<InteractionResponse>,
        mut controls: mpsc::UnboundedReceiver<SteerCommand>,
    ) -> Result<(), ExecutionError> {
        let (mut child, mut stdin, mut stdout) = self.start(Some(&request)).await?;
        Self::initialize(&mut stdin, &mut stdout).await?;
        let mut content = vec![json!({ "type": "text", "text": request.full_prompt() })];
        for image in request.attachments.iter().filter(|a| a.is_image()) {
            let bytes = tokio::fs::read(&image.path).await.map_err(|e| {
                ExecutionError::with_source(format!("Could not read image {}", image.name), e)
            })?;
            if bytes.len() > 10 * 1024 * 1024 {
                return Err(ExecutionError::new(format!(
                    "Image {} exceeds the 10 MiB limit",
                    image.name
                )));
            }
            content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": image.mime_type, "data": STANDARD.encode(bytes) } }));
        }
        let initial_id = uuid::Uuid::new_v4().to_string();
        write(&mut stdin, &json!({ "uuid": initial_id, "type": "user", "message": { "role": "user", "content": content }, "parent_tool_use_id": null, "session_id": request.binding.as_ref().map(|b| &b.native_session_id) })).await?;
        let _ = events.send(ExecutionEvent::Activity(Activity::Working));
        let mut streamed_ids = HashSet::new();
        let mut outstanding_results = 1usize;
        let mut pending_steer: Option<SteerCommand> = None;
        let mut controls_open = true;
        let mut replay_verified = false;
        let (frames, mut incoming) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let frame = read(&mut stdout).await;
                let failed = frame.is_err();
                if frames.send(frame).await.is_err() || failed {
                    break;
                }
            }
        });
        let mut bound = false;
        loop {
            let value = tokio::select! {
                frame = incoming.recv() => frame.ok_or_else(|| ExecutionError::new("Claude disconnected"))??,
                command = controls.recv(), if controls_open && pending_steer.is_none() => {
                    let Some(command) = command else { controls_open = false; continue; };
                    if !replay_verified { let _ = command.result.send(SteerOutcome::Rejected("This Claude version has not verified input acknowledgements. The message remains queued.".into())); continue; }
                    if let Err(error) = crate::steering::validate(&command.input).await { let _ = command.result.send(SteerOutcome::Rejected(error.to_string())); continue; }
                    let mut content = vec![json!({"type":"text", "text":command.input.text_with_files()})];
                    for file in command.input.attachments.iter().filter(|f| f.is_image()) {
                        let bytes = tokio::fs::read(&file.path).await.map_err(|e| ExecutionError::with_source("Could not read steered image", e))?;
                        content.push(json!({"type":"image", "source":{"type":"base64", "media_type":file.mime_type, "data": STANDARD.encode(bytes)}}));
                    }
                    write(&mut stdin, &json!({"type":"user", "uuid":command.input.id.to_string(), "message":{"role":"user", "content":content}, "parent_tool_use_id":null})).await?;
                    outstanding_results += 1;
                    pending_steer = Some(command);
                    continue;
                }
            };
            if value["type"] == "user" {
                if value["uuid"].as_str() == Some(initial_id.as_str()) {
                    replay_verified = true;
                    let _ = events.send(ExecutionEvent::SteeringAvailable(true));
                }
                if pending_steer.as_ref().is_some_and(|command| {
                    value["uuid"].as_str() == Some(command.input.id.to_string().as_str())
                }) {
                    let command = pending_steer.take().expect("matching input");
                    let _ = events.send(ExecutionEvent::InputAccepted(command.input));
                    let _ = command.result.send(SteerOutcome::Accepted);
                }
            }
            if !bound && let Some(session) = value["session_id"].as_str() {
                let _ = events.send(ExecutionEvent::BindingEstablished(ExecutionBinding {
                    adapter_key: "claude".into(),
                    native_session_id: session.into(),
                }));
                bound = true;
            }
            match value["type"].as_str() {
                Some("stream_event") => {
                    let event = &value["event"];
                    match event["type"].as_str() {
                        Some("message_start") => {
                            if let Some(id) = event["message"]["id"].as_str() {
                                streamed_ids.insert(id.to_owned());
                            }
                        }
                        Some("content_block_start") => {
                            let activity = match event["content_block"]["type"].as_str() {
                                Some("thinking") => Activity::Thinking,
                                Some("text") => Activity::Responding,
                                _ => Activity::Working,
                            };
                            let _ = events.send(ExecutionEvent::Activity(activity));
                        }
                        Some("content_block_delta") => match event["delta"]["type"].as_str() {
                            Some("text_delta") => {
                                let _ = events.send(ExecutionEvent::Activity(Activity::Responding));
                                if let Some(text) = event["delta"]["text"].as_str() {
                                    let _ = events.send(ExecutionEvent::TextDelta(text.into()));
                                }
                            }
                            Some("thinking_delta") => {
                                let _ = events.send(ExecutionEvent::Activity(Activity::Thinking));
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                Some("assistant") => {
                    let id = value["message"]["id"].as_str().unwrap_or("");
                    if !streamed_ids.contains(id) {
                        for block in value["message"]["content"].as_array().into_iter().flatten() {
                            if block["type"] == "text"
                                && let Some(text) = block["text"].as_str()
                            {
                                let _ = events.send(ExecutionEvent::TextDelta(text.into()));
                            }
                        }
                    }
                    if let Some(error) = value["error"].as_str() {
                        return Err(ExecutionError::new(format!(
                            "Claude: {error}. Check `claude auth status`."
                        )));
                    }
                }
                Some("control_request") => {
                    handle_interaction(&mut stdin, &value, &events, &mut responses).await?
                }
                Some("result") => {
                    if value["is_error"].as_bool() == Some(true) {
                        return Err(ExecutionError::new(format!(
                            "Claude turn failed: {}",
                            value["result"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value["errors"].to_string())
                        )));
                    }
                    outstanding_results = outstanding_results.saturating_sub(1);
                    if outstanding_results == 0 {
                        if !request.settings.plan_mode
                            && let Some(text) =
                                value["result"].as_str().filter(|s| !s.trim().is_empty())
                        {
                            let _ = events.send(ExecutionEvent::FinalAnswerText(text.to_owned()));
                        }
                        break;
                    }
                }
                Some("system") if value["subtype"] == "error" => {
                    return Err(ExecutionError::new(format!("Claude: {}", value["message"])));
                }
                _ => {}
            }
        }
        if let Some(command) = pending_steer {
            let _ = command.result.send(SteerOutcome::Uncertain(
                "Claude finished without acknowledging the message. Review the conversation."
                    .into(),
            ));
        }
        while let Ok(command) = controls.try_recv() {
            let _ = command.result.send(SteerOutcome::NotRunning);
        }
        drop(stdin);
        if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionAdapter for ClaudeAdapter {
    async fn capabilities(
        &self,
        _provider: Provider,
    ) -> Result<ProviderCapabilities, ExecutionError> {
        let (mut child, mut stdin, mut stdout) = self.start(None).await?;
        let data = Self::initialize(&mut stdin, &mut stdout).await?;
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for model in data["models"].as_array().into_iter().flatten() {
            let id = model["resolvedModel"]
                .as_str()
                .or_else(|| model["value"].as_str())
                .ok_or_else(|| {
                    ExecutionError::new("Claude returned a model without an identifier")
                })?;
            if !seen.insert(id.to_owned()) {
                continue;
            }
            models.push(ModelCapability {
                id: id.into(),
                label: if model["value"] == "default" {
                    let name = model["description"]
                        .as_str()
                        .and_then(|s| s.split('·').next())
                        .unwrap_or(id)
                        .trim();
                    format!("{name} (default)")
                } else {
                    model["displayName"].as_str().unwrap_or(id).into()
                },
                efforts: model["supportedEffortLevels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                default_effort: None,
                images: true,
                is_default: model["value"] == "default",
            });
        }
        drop(stdin);
        if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        if models.is_empty() {
            return Err(ExecutionError::new(
                "Claude Code did not advertise supported models. Update the CLI and check `claude auth status`.",
            ));
        }
        Ok(ProviderCapabilities {
            provider: Provider::Claude,
            models,
        })
    }
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        let (_, responses) = mpsc::unbounded_channel();
        let (_, controls) = mpsc::unbounded_channel();
        self.run(request, events, responses, controls).await
    }
    async fn execute_controlled(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
        controls: mpsc::UnboundedReceiver<SteerCommand>,
    ) -> Result<(), ExecutionError> {
        self.run(request, events, responses, controls).await
    }
    async fn execute_interactive(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
        responses: mpsc::UnboundedReceiver<InteractionResponse>,
    ) -> Result<(), ExecutionError> {
        let (_, controls) = mpsc::unbounded_channel();
        self.run(request, events, responses, controls).await
    }
}

async fn handle_interaction(
    stdin: &mut ChildStdin,
    value: &Value,
    events: &mpsc::UnboundedSender<ExecutionEvent>,
    responses: &mut mpsc::UnboundedReceiver<InteractionResponse>,
) -> Result<(), ExecutionError> {
    let id = value["request_id"]
        .as_str()
        .ok_or_else(|| ExecutionError::new("Claude interaction has no request ID"))?
        .to_owned();
    let request = &value["request"];
    if request["subtype"] != "can_use_tool" {
        write(stdin, &json!({ "type": "control_response", "response": { "subtype": "error", "request_id": id, "error": "Unsupported interaction" } })).await?;
        return Err(ExecutionError::new(format!(
            "Unsupported Claude interaction: {}",
            request["subtype"]
        )));
    }
    let input = &request["input"];
    let tool = request["tool_name"].as_str().unwrap_or("Tool");
    let kind = match tool {
        "AskUserQuestion" => {
            InteractionKind::Questions(crate::codex::parse_questions(&input["questions"]))
        }
        "ExitPlanMode" => InteractionKind::LeavePlan {
            detail: input["plan"]
                .as_str()
                .unwrap_or("Allow Claude to leave Plan mode and continue with this plan?")
                .into(),
        },
        _ => InteractionKind::Approval(crate::approvals::present(tool, input, false)),
    };
    if matches!(&kind, InteractionKind::Questions(questions) if questions.is_empty()) {
        return Err(ExecutionError::new("Claude sent an empty question request"));
    }
    let questions = matches!(kind, InteractionKind::Questions(_));
    let _ = events.send(ExecutionEvent::Activity(Activity::Waiting));
    events
        .send(ExecutionEvent::Interaction(Interaction {
            id: id.clone(),
            title: if questions {
                "A question from Claude".into()
            } else {
                format!("Claude requests {tool}")
            },
            kind,
        }))
        .map_err(|_| {
            ExecutionError::new("The chat closed before the interaction could be shown")
        })?;
    let answer = loop {
        let answer = responses.recv().await.ok_or_else(|| {
            ExecutionError::new("The chat closed while Claude was waiting for an answer")
        })?;
        if answer.id == id {
            break answer;
        }
    };
    let response = if answer.approved || questions {
        let mut updated = input.clone();
        if questions {
            let answers: serde_json::Map<String, Value> = answer
                .answers
                .into_iter()
                .map(|(id, values)| (id, json!(values.join(", "))))
                .collect();
            updated["answers"] = json!(answers);
        }
        json!({ "behavior": "allow", "updatedInput": updated })
    } else {
        json!({ "behavior": "deny", "message": "The user declined this request." })
    };
    write(stdin, &json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id, "response": response } })).await?;
    let _ = events.send(ExecutionEvent::Activity(Activity::Working));
    Ok(())
}

async fn write(stdin: &mut ChildStdin, value: &Value) -> Result<(), ExecutionError> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|e| ExecutionError::with_source("Could not encode Claude input", e))?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .map_err(|e| ExecutionError::with_source("Could not write to Claude", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| ExecutionError::with_source("Could not flush Claude input", e))
}
async fn read(stdout: &mut BufReader<ChildStdout>) -> Result<Value, ExecutionError> {
    let mut line = String::new();
    if stdout
        .read_line(&mut line)
        .await
        .map_err(|e| ExecutionError::with_source("Could not read Claude output", e))?
        == 0
    {
        return Err(ExecutionError::new(
            "Claude closed its output before completing. Check `claude auth status`.",
        ));
    }
    serde_json::from_str(&line)
        .map_err(|e| ExecutionError::with_source("Claude returned invalid JSON", e))
}
