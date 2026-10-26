#![cfg(unix)]
use smaller_talk_application::*;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::*;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

fn executable(root: &Path, name: &str, source: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let file = root.join(name);
    std::fs::write(&file, source).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    file
}
fn request(root: &Path, provider: Provider) -> CompactionRequest {
    CompactionRequest {
        chat_id: Uuid::new_v4(),
        binding: ExecutionBinding {
            adapter_key: provider.key().into(),
            native_session_id: "existing-native".into(),
            instructions_fingerprint: None,
        },
        workspace: root.into(),
        settings: ExecutionSettings {
            provider,
            ..Default::default()
        },
        instructions: Default::default(),
    }
}
const CODEX: &str = r#"#!/usr/bin/env python3
import json,sys,pathlib,time
send=lambda v:print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line); m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='thread/resume':
  assert r['params']['threadId']=='existing-native'
  send({'id':r['id'],'result':{'thread':{'id':'existing-native'}}})
 elif m=='thread/compact/start':
  assert r['params']=={'threadId':'existing-native'}
  send({'id':r['id'],'result':{}})
  pathlib.Path('accepted').touch()
  while not pathlib.Path('release').exists():time.sleep(.01)
  send({'method':'item/completed','params':{'threadId':'existing-native','item':{'type':'contextCompaction'}}})
  send({'method':'turn/completed','params':{'threadId':'existing-native','turn':{'status':'completed'}}})
 elif m=='turn/start' or m=='thread/start':raise Exception('must only compact existing context')
"#;

#[tokio::test]
async fn codex_compaction_waits_for_native_completion_after_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(temp.path(), "codex", CODEX);
    let req = request(temp.path(), Provider::Codex);
    let task = tokio::spawn(async move { CodexAdapter::new(exe).compact(req).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !temp.path().join("accepted").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!task.is_finished());
    std::fs::write(temp.path().join("release"), "").unwrap();
    assert_eq!(task.await.unwrap().unwrap(), CompactionOutcome::Compacted);
}

#[tokio::test]
async fn codex_compaction_timeout_reports_uncertainty() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(temp.path(), "codex", CODEX);
    let req = request(temp.path(), Provider::Codex);
    let task = tokio::spawn(async move { CodexAdapter::new(exe).compact(req).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !temp.path().join("accepted").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(181)).await;
    tokio::time::resume();
    let error = task.await.unwrap().unwrap_err().to_string();
    assert!(
        error.contains("timed out") && error.contains("uncertain"),
        "{error}"
    );
}

#[tokio::test]
async fn codex_unsupported_and_disconnect_do_not_fall_back_to_a_prompt() {
    for response in [
        "send({'id':r['id'],'error':{'code':-32601,'message':'Unsupported'}})",
        "sys.exit(0)",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = format!(
            "#!/usr/bin/env python3\nimport json,sys\nsend=lambda v:print(json.dumps(v),flush=True)\nfor line in sys.stdin:\n r=json.loads(line);m=r.get('method')\n if m=='thread/compact/start':{response}\n elif m in ['initialize','thread/resume']:send({{'id':r['id'],'result':{{}}}})\n elif m=='turn/start':raise Exception('fallback forbidden')\n"
        );
        let exe = executable(temp.path(), "codex", &source);
        assert!(
            CodexAdapter::new(exe)
                .compact(request(temp.path(), Provider::Codex))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn claude_uses_bare_native_command_and_distinguishes_compacted_noop_and_error() {
    for (response, expected) in [
        (
            "send({'type':'system','subtype':'compact_boundary','compact_metadata':{'trigger':'manual'}});send({'type':'result','is_error':False,'result':''})",
            Some(CompactionOutcome::Compacted),
        ),
        (
            "send({'type':'result','is_error':False,'result':'Not enough messages to compact.'})",
            Some(CompactionOutcome::NothingToCompact(
                "Not enough messages to compact.".into(),
            )),
        ),
        (
            "send({'type':'result','is_error':True,'result':'Command unavailable'})",
            None,
        ),
        (
            "send({'type':'result','is_error':False,'result':'The command did not run'})",
            None,
        ),
        ("sys.exit(0)", None),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = format!(
            "#!/usr/bin/env python3\nimport json,sys\nassert '--resume' in sys.argv and sys.argv[sys.argv.index('--resume')+1]=='existing-native'\nsend=lambda v:print(json.dumps(v),flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n if r['type']=='control_request':send({{'type':'control_response','response':{{'request_id':r['request_id'],'subtype':'success','response':{{}}}}}})\n elif r['type']=='user':\n  assert r['message']['content']=='/compact'\n  {response}\n"
        );
        let exe = executable(temp.path(), "claude", &source);
        let actual = ClaudeAdapter::new(exe)
            .compact(request(temp.path(), Provider::Claude))
            .await;
        match expected {
            Some(expected) => assert_eq!(actual.unwrap(), expected),
            None => assert!(actual.is_err()),
        }
    }
}
