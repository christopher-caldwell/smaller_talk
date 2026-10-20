#![cfg(unix)]
use smaller_talk_application::{ExecutionAdapter, ExecutionEvent, ExecutionRequest};
use smaller_talk_domain::*;
use smaller_talk_infrastructure::{ClaudeAdapter, CodexAdapter};
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;

fn executable(root: &std::path::Path, name: &str, source: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
fn request(root: &std::path::Path, provider: Provider) -> ExecutionRequest {
    let image = root.join("image.png");
    std::fs::write(&image, b"image-data").unwrap();
    ExecutionRequest {
        chat_id: Uuid::new_v4(),
        binding: None,
        workspace: root.into(),
        prompt: "Current question".into(),
        settings: ExecutionSettings {
            provider,
            model: Some("fixture-model".into()),
            effort: Some("low".into()),
            plan_mode: true,
        },
        attachments: vec![Attachment {
            id: "image".into(),
            name: "image.png".into(),
            path: image,
            mime_type: "image/png".into(),
            byte_len: 10,
            preview_path: None,
        }],
        prior_context: Some("Saved visible context".into()),
    }
}

async fn run(
    agent: impl ExecutionAdapter + 'static,
    request: ExecutionRequest,
) -> Vec<ExecutionEvent> {
    let (events, mut stream) = mpsc::unbounded_channel();
    let (answers, receiver) = mpsc::unbounded_channel();
    let task =
        tokio::spawn(async move { agent.execute_interactive(request, events, receiver).await });
    let mut observed = vec![];
    while let Some(event) = tokio::time::timeout(Duration::from_secs(15), stream.recv())
        .await
        .unwrap()
    {
        if let ExecutionEvent::Interaction(interaction) = &event {
            let mut response = InteractionResponse {
                id: interaction.id.clone(),
                approved: true,
                ..Default::default()
            };
            if let InteractionKind::Questions(questions) = &interaction.kind {
                for question in questions {
                    response
                        .answers
                        .insert(question.id.clone(), vec!["Blue".into()]);
                }
            }
            answers.send(response).unwrap();
        }
        observed.push(event);
    }
    task.await.unwrap().unwrap();
    observed
}

#[tokio::test]
async fn codex_native_images_plan_settings_questions_and_approvals_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "codex-fixture",
        r##"#!/usr/bin/env python3
import json,sys
send=lambda value:print(json.dumps(value),flush=True)
read=lambda:json.loads(sys.stdin.readline())
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':
  assert r['params']['capabilities']['experimentalApi']
  send({'id':r['id'],'result':{}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'native-codex'}}})
 elif m=='turn/start':
  p=r['params'];assert p['model']=='fixture-model';assert p['effort']=='low'
  assert p['collaborationMode']['mode']=='plan'
  assert p['input'][1]['type']=='localImage'
  assert 'Saved visible context' in p['input'][0]['text']
  send({'id':r['id'],'result':{'turn':{'id':'turn'}}})
  send({'method':'item/reasoning/summaryTextDelta','params':{'delta':'Thinking'}})
  send({'id':'q','method':'item/tool/requestUserInput','params':{'questions':[{'id':'color','question':'Which color?','options':[{'label':'Blue'}]}]}})
  a=read();assert a['id']=='q';assert a['result']['answers']['color']['answers']==['Blue']
  send({'id':'approve','method':'item/commandExecution/requestApproval','params':{'command':'read a file'}})
  a=read();assert a['result']['decision']=='accept'
  send({'method':'item/agentMessage/delta','params':{'threadId':'native-codex','delta':'Answered Blue'}})
  send({'method':'turn/completed','params':{'threadId':'native-codex','turn':{'status':'completed'}}})
"##,
    );
    let events = run(
        CodexAdapter::new(script),
        request(temp.path(), Provider::Codex),
    )
    .await;
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ExecutionEvent::Interaction(_)))
            .count(),
        2
    );
    assert!(events.contains(&ExecutionEvent::Activity(Activity::Thinking)));
    assert!(events.contains(&ExecutionEvent::TextDelta("Answered Blue".into())));
}

#[tokio::test]
async fn claude_native_images_metadata_questions_plan_exit_and_stream_deduplication() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "claude-fixture",
        r##"#!/usr/bin/env python3
import base64,json,sys
send=lambda value:print(json.dumps(value),flush=True)
read=lambda:json.loads(sys.stdin.readline())
for line in sys.stdin:
 r=json.loads(line)
 if r['type']=='control_request':
  send({'type':'control_response','response':{'subtype':'success','request_id':r['request_id'],'response':{'models':[{'value':'default','resolvedModel':'fixture-model','displayName':'Fixture model','supportsEffort':True,'supportedEffortLevels':['low','high']}]}}})
 elif r['type']=='user':
  assert '--permission-mode' in sys.argv and sys.argv[sys.argv.index('--permission-mode')+1]=='plan'
  c=r['message']['content'];assert 'Saved visible context' in c[0]['text'];assert c[1]['source']['media_type']=='image/png';assert base64.b64decode(c[1]['source']['data'])==b'image-data'
  send({'type':'system','subtype':'init','session_id':'native-claude'})
  send({'type':'control_request','request_id':'q','request':{'subtype':'can_use_tool','tool_name':'AskUserQuestion','input':{'questions':[{'question':'Which color?','options':[{'label':'Blue'}]}]}}})
  a=read();assert a['response']['response']['updatedInput']['answers']['Which color?']=='Blue'
  send({'type':'control_request','request_id':'plan','request':{'subtype':'can_use_tool','tool_name':'ExitPlanMode','input':{'plan':'Use Blue'}}})
  a=read();assert a['response']['response']['behavior']=='allow'
  send({'type':'stream_event','event':{'type':'message_start','message':{'id':'answer'}}})
  send({'type':'stream_event','event':{'type':'content_block_start','content_block':{'type':'thinking'}}})
  send({'type':'stream_event','event':{'type':'content_block_delta','delta':{'type':'text_delta','text':'Answered Blue'}}})
  send({'type':'assistant','message':{'id':'answer','content':[{'type':'text','text':'Answered Blue'}]}})
  send({'type':'result','is_error':False})
"##,
    );
    let agent = ClaudeAdapter::new(&script);
    let caps = agent.capabilities(Provider::Claude).await.unwrap();
    assert_eq!(caps.models[0].id, "fixture-model");
    assert_eq!(caps.models[0].efforts, ["low", "high"]);
    let events = run(agent, request(temp.path(), Provider::Claude)).await;
    let output: String = events
        .iter()
        .filter_map(|e| {
            if let ExecutionEvent::TextDelta(t) = e {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(output, "Answered Blue");
    assert!(events.contains(&ExecutionEvent::Activity(Activity::Thinking)));
    assert!(events.iter().any(|e| matches!(
        e,
        ExecutionEvent::Interaction(Interaction {
            kind: InteractionKind::LeavePlan { .. },
            ..
        })
    )));
}

#[tokio::test]
async fn codex_discovery_uses_advertised_efforts_and_filters_hidden_models() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "catalog-fixture",
        r##"#!/usr/bin/env python3
import json,sys
for line in sys.stdin:
 r=json.loads(line)
 if r.get('method')=='initialize':print(json.dumps({'id':r['id'],'result':{}}),flush=True)
 elif r.get('method')=='model/list':print(json.dumps({'id':r['id'],'result':{'data':[{'id':'visible','model':'visible','displayName':'Visible','isDefault':True,'supportedReasoningEfforts':[{'reasoningEffort':'low'},{'reasoningEffort':'medium'}],'defaultReasoningEffort':'medium','inputModalities':['text']},{'id':'hidden','hidden':True}],'nextCursor':None}}),flush=True)
"##,
    );
    let caps = CodexAdapter::new(script)
        .capabilities(Provider::Codex)
        .await
        .unwrap();
    assert_eq!(caps.models.len(), 1);
    assert!(!caps.models[0].images);
    assert_eq!(caps.models[0].default_effort.as_deref(), Some("medium"));
}

#[tokio::test]
async fn completed_plan_replaces_its_streamed_draft() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "plan-fixture",
        r##"#!/usr/bin/env python3
import json,sys
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'thread'}}})
 elif m=='turn/start':
  send({'id':r['id'],'result':{'turn':{'id':'turn'}}})
  send({'method':'item/plan/delta','params':{'threadId':'thread','itemId':'plan','delta':'Draft plan'}})
  send({'method':'item/completed','params':{'item':{'id':'plan','type':'plan','text':'Final authoritative plan'}}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'status':'completed'}}})
"##,
    );
    let events = run(
        CodexAdapter::new(script),
        request(temp.path(), Provider::Codex),
    )
    .await;
    assert!(events.contains(&ExecutionEvent::TextDelta("Draft plan".into())));
    assert!(events.contains(&ExecutionEvent::TextSnapshot(
        "Final authoritative plan".into()
    )));
}
