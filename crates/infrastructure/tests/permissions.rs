#![cfg(unix)]

use smaller_talk_application::{ExecutionAdapter, ExecutionEvent, ExecutionRequest};
use smaller_talk_domain::*;
use smaller_talk_infrastructure::{ClaudeAdapter, CodexAdapter};
use std::{os::unix::fs::PermissionsExt, path::Path};
use tokio::sync::mpsc;
use uuid::Uuid;

fn request(
    root: &Path,
    provider: Provider,
    mode: PermissionMode,
    resume: bool,
    plan: bool,
) -> ExecutionRequest {
    let other = if provider == Provider::Claude {
        Provider::Codex
    } else {
        Provider::Claude
    };
    ExecutionRequest {
        chat_id: Uuid::new_v4(),
        workspace: root.into(),
        prompt: "Synthetic permission check".into(),
        settings: ExecutionSettings {
            provider,
            model: Some("fixture".into()),
            plan_mode: plan,
            permissions: [(provider, mode), (other, PermissionMode::FullAccess)].into(),
            ..Default::default()
        },
        binding: resume.then(|| ExecutionBinding {
            adapter_key: provider.key().into(),
            native_session_id: "native".into(),
            instructions_fingerprint: None,
        }),
        attachments: vec![],
        prior_context: None,
        skills: vec![],
        retry_inputs: vec![],
        instructions: Default::default(),
    }
}

async fn execute(adapter: impl ExecutionAdapter, input: ExecutionRequest) -> Vec<ExecutionEvent> {
    let (sender, mut events) = mpsc::unbounded_channel();
    let (_responses, receiver) = mpsc::unbounded_channel();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        adapter.execute_interactive(input, sender, receiver),
    )
    .await
    .unwrap()
    .unwrap();
    let mut collected = vec![];
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    collected
}

fn script(root: &Path, content: String) -> std::path::PathBuf {
    let path = root.join("provider-fixture");
    std::fs::write(&path, content).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn codex_permissions_are_explicit_on_start_resume_and_every_turn() {
    for (mode, expected) in [
        (PermissionMode::ProviderDefault, None),
        (
            PermissionMode::Supervised,
            Some(("untrusted", "user", "read-only", "readOnly")),
        ),
        (
            PermissionMode::AcceptEdits,
            Some(("on-request", "user", "workspace-write", "workspaceWrite")),
        ),
        (
            PermissionMode::Auto,
            Some((
                "on-request",
                "auto_review",
                "workspace-write",
                "workspaceWrite",
            )),
        ),
        (
            PermissionMode::FullAccess,
            Some(("never", "user", "danger-full-access", "dangerFullAccess")),
        ),
    ] {
        for resume in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let expected = serde_json::to_string(&expected).unwrap();
            let source = format!(r#"#!/usr/bin/env python3
import json,sys
expected={expected}
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method');p=r.get('params',{{}})
 if m=='initialize':send({{'id':r['id'],'result':{{}}}})
 elif m in ['thread/start','thread/resume','turn/start']:
  if expected is None:
   assert all(k not in p for k in ['approvalPolicy','approvalsReviewer','sandbox','sandboxPolicy'])
  else:
   assert p['approvalPolicy']==expected[0] and p['approvalsReviewer']==expected[1],p
   if m=='turn/start':assert p['sandboxPolicy']=={{'type':expected[3]}},p
   else:assert p['sandbox']==expected[2],p
  if m=='turn/start':
   assert p['collaborationMode']['mode']=='plan'
   send({{'id':r['id'],'result':{{'turn':{{'id':'turn'}}}}}})
   send({{'method':'item/agentMessage/delta','params':{{'threadId':'native','delta':'Done'}}}})
   send({{'method':'turn/completed','params':{{'threadId':'native','turn':{{'status':'completed'}}}}}})
  else:send({{'id':r['id'],'result':{{'thread':{{'id':'native'}}}}}})
"#).replace("expected=null", "expected=None");
            let path = script(dir.path(), source);
            execute(
                CodexAdapter::new(path),
                request(dir.path(), Provider::Codex, mode, resume, true),
            )
            .await;
        }
    }
}

#[tokio::test]
async fn claude_uses_native_permission_flags_and_reports_effective_mode() {
    for (mode, flag) in [
        (PermissionMode::ProviderDefault, None),
        (PermissionMode::Supervised, Some("default")),
        (PermissionMode::AcceptEdits, Some("acceptEdits")),
        (PermissionMode::Auto, Some("auto")),
        (PermissionMode::FullAccess, Some("bypassPermissions")),
    ] {
        for plan in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let flag = if plan { Some("plan") } else { flag };
            let expected = serde_json::to_string(&flag).unwrap();
            let path = script(dir.path(), format!(r#"#!/usr/bin/env python3
import json,sys
expected={expected}
if expected is None:assert '--permission-mode' not in sys.argv
else:assert sys.argv[sys.argv.index('--permission-mode')+1]==expected
assert sys.argv[sys.argv.index('--resume')+1]=='native'
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line)
 if r['type']=='control_request':send({{'type':'control_response','response':{{'subtype':'success','request_id':r['request_id'],'response':{{}}}}}})
 elif r['type']=='user':
  send({{'type':'system','subtype':'init','session_id':'native','permissionMode':'default'}})
  send({{'type':'assistant','message':{{'id':'answer','content':[{{'type':'text','text':'Done'}}]}}}})
  send({{'type':'result','is_error':False}})
"#).replace("expected=null", "expected=None"));
            let events = execute(
                ClaudeAdapter::new(path),
                request(dir.path(), Provider::Claude, mode, true, plan),
            )
            .await;
            assert!(events.contains(&ExecutionEvent::Permissions(PermissionMode::Supervised)));
        }
    }
}
