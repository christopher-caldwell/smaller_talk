use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use serde_json::{Value, json};
use smaller_talk_application::{
    ExecutionAdapter, ExecutionError, ExecutionEvent, ExecutionRequest,
};
use smaller_talk_domain::ExecutionBinding;
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

    async fn run_turn(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
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
                    "experimentalApi": false,
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
        let mut thread_params = json!({ "cwd": request.workspace.to_string_lossy() });
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

        send_request(
            &mut stdin,
            3,
            "turn/start",
            json!({
                "threadId": thread_id,
                "cwd": request.workspace.to_string_lossy(),
                "input": [{ "type": "text", "text": request.prompt, "text_elements": [] }]
            }),
        )
        .await?;

        let mut turn_response_received = false;
        let mut turn_completed = false;
        while !turn_completed {
            let value = read_message(&mut stdout).await?;
            if value.get("method").is_some() && value.get("id").is_some() {
                handle_server_request(&mut stdin, &value).await?;
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
                Some("item/agentMessage/delta") => {
                    let params = &value["params"];
                    if params["threadId"].as_str() == Some(thread_id.as_str()) {
                        if let Some(delta) = params["delta"].as_str() {
                            events
                                .send(ExecutionEvent::TextDelta(delta.to_owned()))
                                .map_err(|error| {
                                    ExecutionError::with_source(
                                        format!("conversation stopped while Codex was responding: {error}"),
                                        error,
                                    )
                                })?;
                        }
                    }
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
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError> {
        self.run_turn(request, events).await
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
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or_default();
    let result = match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "applyPatchApproval"
        | "execCommandApproval" => Some(json!({ "decision": "decline" })),
        "item/permissions/requestApproval" => Some(json!({ "permissions": {}, "scope": "turn" })),
        "item/tool/requestUserInput" => Some(json!({ "answers": {} })),
        _ => None,
    };
    let response = if let Some(result) = result {
        json!({ "id": id, "result": result })
    } else {
        json!({
            "id": id,
            "error": { "code": -32601, "message": "Smaller Talk does not support this Codex interaction" }
        })
    };
    write_message(stdin, &response).await
}

fn rpc_error(value: &Value) -> ExecutionError {
    let error = &value["error"];
    let message = error["message"]
        .as_str()
        .or_else(|| error.as_str())
        .unwrap_or("Codex app-server returned an unspecified error");
    ExecutionError::new(message.to_owned())
}

async fn read_stderr(stderr: tokio::process::ChildStderr) -> String {
    let mut stderr = stderr.take(16 * 1024);
    let mut output = String::new();
    let _ = stderr.read_to_string(&mut output).await;
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
        }
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
        assert_eq!(
            receiver.recv().await,
            Some(ExecutionEvent::TextDelta("streamed answer".into()))
        );
        assert!(receiver.recv().await.is_none());
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
