#![cfg(unix)]

use smaller_talk_application::{
    CompactionOutcome, CompactionRequest, ExecutionAdapter, ExecutionEvent, ExecutionRequest,
    ProviderControl,
};
use smaller_talk_domain::{Attachment, ExecutionBinding, ExecutionSettings, Provider};
use smaller_talk_infrastructure::{AgentRouter, ClaudeAdapter, CodexAdapter};
use std::{
    fs, os::unix::fs::PermissionsExt, path::Path, process::Stdio, sync::Arc, time::Duration,
};
use tokio::{process::Command, sync::mpsc};
use uuid::Uuid;

const SCRIPT: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, sys

def send(value):
    print(json.dumps(value), flush=True)

root = pathlib.Path(__file__).parent
pid = os.getpid()
args = sys.argv[1:]
is_probe = "--model" not in args
with (root / "starts").open("a") as log:
    log.write(f"{pid}\n")

if "--resume" in args:
    session = args[args.index("--resume") + 1]
    state = root / session
    previous = int(state.read_text())
    try:
        os.kill(previous, 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError("The previous process still owns this session")
else:
    session = f"fixture-{pid}"
(root / session).write_text(str(pid))
turns = 0
for line in sys.stdin:
    frame = json.loads(line)
    if frame["type"] == "control_request":
        catalog = "Discovery process" if is_probe else "Turn process"
        send({"type":"control_response","response":{"request_id":frame["request_id"],"subtype":"success","response":{
          "models":[{"value":"fixture-model","displayName":catalog,"supportedEffortLevels":["low","high"]}]}}})
        continue
    prompt = frame["message"]["content"]
    text = prompt[0]["text"] if isinstance(prompt, list) else prompt
    if text == "/compact":
        send({"type":"system","subtype":"compact_boundary"})
        send({"type":"result","result":"compacted","is_error":False})
        continue
    send({"type":"user","uuid":frame["uuid"],"session_id":session})
    if text == "crash":
        sys.exit(1)
    turns += 1
    answer = f"pid={pid},connector={'ready' if turns > 1 else 'connecting'}"
    send({"type":"assistant","session_id":session,"message":{"id":"visible","content":[{"type":"text","text":answer}]}})
    send({"type":"result","session_id":session,"result":answer,"is_error":False})
"#;

struct Fixture {
    folder: tempfile::TempDir,
    router: Arc<AgentRouter>,
}

impl Fixture {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("claude-fixture");
        fs::write(&executable, SCRIPT).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let router = AgentRouter::new(CodexAdapter::new("unused"), ClaudeAdapter::new(&executable));
        Self { folder, router }
    }

    fn root(&self) -> &Path {
        self.folder.path()
    }

    fn request(&self) -> ExecutionRequest {
        ExecutionRequest {
            chat_id: Uuid::new_v4(),
            workspace: self.root().into(),
            binding: None,
            prompt: "test turn".into(),
            settings: ExecutionSettings {
                provider: Provider::Claude,
                model: Some("fixture-model".into()),
                effort: Some("low".into()),
                ..Default::default()
            },
            instructions: Default::default(),
            attachments: Vec::new(),
            skills: Vec::new(),
            retry_inputs: Vec::new(),
            prior_context: None,
        }
    }

    fn processes(&self) -> Vec<u32> {
        fs::read_to_string(self.root().join("starts"))
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect()
    }

    async fn turn(&self, request: ExecutionRequest) -> (ExecutionBinding, String) {
        let (events, mut stream) = mpsc::unbounded_channel();
        self.router.execute(request, events).await.unwrap();
        let mut binding = None;
        let mut answer = String::new();
        while let Ok(event) = stream.try_recv() {
            match event {
                ExecutionEvent::BindingEstablished(value) => binding = Some(value),
                ExecutionEvent::TextDelta(text) => answer.push_str(&text),
                _ => {}
            }
        }
        (binding.unwrap(), answer)
    }
}

async fn assert_exited(pid: u32) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("the idle Claude child should exit");
}

#[tokio::test]
async fn reuses_connection_then_expires_and_starts_only_one_process_after_five_minutes() {
    let fixture = Fixture::new();
    let router = &fixture.router;
    assert_eq!(
        router.capabilities(Provider::Claude).await.unwrap().models[0].label,
        "Discovery process"
    );
    let mut request = fixture.request();
    let (binding, first) = fixture.turn(request.clone()).await;
    request.binding = Some(binding);
    let (_, second) = fixture.turn(request.clone()).await;
    let processes = fixture.processes();
    assert_eq!(processes.len(), 2);
    assert_eq!(first, format!("pid={},connector=connecting", processes[1]));
    assert_eq!(second, format!("pid={},connector=ready", processes[1]));

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::time::resume();
    assert_exited(processes[1]).await;
    assert_eq!(
        router.capabilities(Provider::Claude).await.unwrap().models[0].label,
        "Turn process"
    );
    assert_eq!(fixture.processes().len(), 2);
    let (_, next) = fixture.turn(request).await;
    assert!(next.ends_with("connector=connecting"));
    assert_eq!(fixture.processes().len(), 3);
    assert_eq!(
        router.refresh(Provider::Claude).await.unwrap().models[0].label,
        "Discovery process"
    );
    assert_eq!(fixture.processes().len(), 4);
}

#[tokio::test]
async fn startup_changes_compaction_and_fresh_sessions_reap_old_children() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    let (binding, _) = fixture.turn(request.clone()).await;
    request.binding = Some(binding.clone());

    request.settings.effort = Some("high".into());
    fixture.turn(request.clone()).await;
    request.instructions.text = "new instructions".into();
    fixture.turn(request.clone()).await;
    let workspace = fixture.root().join("new-workspace");
    fs::create_dir(&workspace).unwrap();
    request.workspace = workspace.clone();
    fixture.turn(request.clone()).await;
    let file = workspace.join("sample.txt");
    fs::write(&file, "sample").unwrap();
    request.attachments.push(Attachment {
        id: "file".into(),
        name: "sample.txt".into(),
        path: file,
        mime_type: "text/plain".into(),
        byte_len: 6,
        preview_path: None,
    });
    fixture.turn(request.clone()).await;
    assert_eq!(fixture.processes().len(), 5);

    let outcome = fixture
        .router
        .compact(CompactionRequest {
            chat_id: request.chat_id,
            binding: binding.clone(),
            workspace,
            settings: request.settings.clone(),
            instructions: request.instructions.clone(),
        })
        .await
        .unwrap();
    assert_eq!(outcome, CompactionOutcome::Compacted);
    assert_eq!(fixture.processes().len(), 6);
    fixture.turn(request.clone()).await;
    let previous_pid = *fixture.processes().last().unwrap();
    request.binding = None; // The application clears the binding for a fresh provider session.
    let (fresh, _) = fixture.turn(request).await;
    assert_ne!(fresh.native_session_id, binding.native_session_id);
    assert_exited(previous_pid).await;
}

#[tokio::test]
async fn keeps_only_the_most_recent_chat_warm_and_resumes_after_a_crash() {
    let fixture = Fixture::new();
    let mut first = fixture.request();
    let (binding, _) = fixture.turn(first.clone()).await;
    first.binding = Some(binding);
    let old = fixture.processes()[0];
    fixture.turn(fixture.request()).await;
    assert_exited(old).await;

    fixture.turn(first.clone()).await;
    let cached = *fixture.processes().last().unwrap();
    let (events, _stream) = mpsc::unbounded_channel();
    let mut crash = first.clone();
    crash.prompt = "crash".into();
    assert!(fixture.router.execute(crash, events).await.is_err());
    assert_exited(cached).await;
    fixture.turn(first).await;
    assert_eq!(fixture.processes().len(), 4);
}
