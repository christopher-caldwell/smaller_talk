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
        skills: Vec::new(),
        retry_inputs: Vec::new(),
        instructions: InstructionSnapshot {
            text: "APP_RULE_FIXTURE".into(),
            ..Default::default()
        },
        chat_id: Uuid::new_v4(),
        binding: None,
        workspace: root.into(),
        prompt: "Current question".into(),
        settings: ExecutionSettings {
            provider,
            model: Some("fixture-model".into()),
            effort: Some("low".into()),
            plan_mode: true,
            permissions: Default::default(),
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
 elif m=='thread/start':
  assert r['params']['developerInstructions']=='APP_RULE_FIXTURE'
  send({'id':r['id'],'result':{'thread':{'id':'native-codex'}}})
 elif m=='turn/start':
  p=r['params'];assert p['model']=='fixture-model';assert p['effort']=='low'
  assert p['collaborationMode']['mode']=='plan'
  assert p['collaborationMode']['settings']['developer_instructions'] is None
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
async fn codex_completed_text_items_keep_markdown_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "codex-item-boundaries",
        r##"#!/usr/bin/env python3
import json,sys
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'thread'}}})
 elif m=='turn/start':
  send({'id':r['id'],'result':{'turn':{'id':'turn'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'first','delta':'First paragraph.'}})
  send({'method':'item/completed','params':{'item':{'id':'first','type':'agentMessage','text':'First paragraph.'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'second','delta':'# Final answer'}})
  send({'method':'item/completed','params':{'item':{'id':'second','type':'agentMessage','text':'# Final answer'}}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'status':'completed'}}})
"##,
    );
    let events = run(
        CodexAdapter::new(script),
        request(temp.path(), Provider::Codex),
    )
    .await;
    assert!(events.contains(&ExecutionEvent::TextSnapshot(
        "First paragraph.\n\n# Final answer".into()
    )));
    let mut visible = String::new();
    let mut heading_streamed_with_boundary = false;
    for event in &events {
        match event {
            ExecutionEvent::TextDelta(delta) => visible.push_str(delta),
            ExecutionEvent::TextSnapshot(snapshot) => visible.clone_from(snapshot),
            _ => {}
        }
        if visible.contains("# Final answer") {
            heading_streamed_with_boundary = visible.contains("First paragraph.\n\n# Final answer");
            break;
        }
    }
    assert!(heading_streamed_with_boundary, "{visible:?}");
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
  assert sys.argv[sys.argv.index('--append-system-prompt')+1]=='APP_RULE_FIXTURE'
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

#[tokio::test]
async fn codex_steering_uses_expected_turn_and_structured_images() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "steer-codex",
        r##"#!/usr/bin/env python3
import json,sys
send=lambda v:print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'thread'}}})
 elif m=='turn/start':
  send({'id':r['id'],'result':{'turn':{'id':'native-turn'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'before','delta':'Before'}})
 elif m=='turn/steer':
  assert r['params']['expectedTurnId']=='native-turn'
  assert r['params']['input'][1]['type']=='localImage'
  send({'id':r['id'],'result':{'turnId':'native-turn'}})
  send({'method':'item/completed','params':{'item':{'id':'before','type':'agentMessage','text':'Before'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'after','delta':'After'}})
  send({'method':'item/completed','params':{'item':{'id':'after','type':'agentMessage','text':'After'}}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'status':'completed'}}})
"##,
    );
    verify_steering(
        CodexAdapter::new(script),
        request(temp.path(), Provider::Codex),
    )
    .await;
}

#[tokio::test]
async fn codex_skill_discovery_failure_rejects_steering_and_keeps_the_turn_running() {
    use smaller_talk_application::{SteerCommand, SteerInput, SteerOutcome};

    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "steer-discovery-error",
        r##"#!/usr/bin/env python3
import json,sys
send=lambda v:print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='skills/list':send({'id':r['id'],'error':{'code':-32603,'message':'Synthetic discovery failure'}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'thread'}}})
 elif m=='turn/start':
  send({'id':r['id'],'result':{'turn':{'id':'native-turn'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'before','delta':'Before'}})
 elif m=='turn/steer':
  assert r['params']['expectedTurnId']=='native-turn'
  assert r['params']['input']==[{'type':'text','text':'Continue','text_elements':[]}]
  send({'id':r['id'],'result':{'turnId':'native-turn'}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','itemId':'after','delta':'After'}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'status':'completed'}}})
"##,
    );
    let request = request(temp.path(), Provider::Codex);
    let workspace = request.workspace.clone();
    let (events, mut receiver) = mpsc::unbounded_channel();
    let (_, responses) = mpsc::unbounded_channel();
    let (controls, control_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        CodexAdapter::new(script)
            .execute_controlled(request, events, responses, control_rx)
            .await
    });
    let accepted_id = Uuid::new_v4();
    let mut rejected = false;
    let mut accepted = false;
    let mut continued = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = receiver.recv().await {
            match event {
                ExecutionEvent::TextDelta(text) if text == "Before" => {
                    let (result, response) = tokio::sync::oneshot::channel();
                    controls
                        .send(SteerCommand {
                            input: SteerInput {
                                id: Uuid::new_v4(),
                                prompt: "$review".into(),
                                attachments: vec![],
                                skills: vec![SkillMention {
                                    range: 0..7,
                                    reference: SkillReference {
                                        provider: Provider::Codex,
                                        name: "review".into(),
                                        path: workspace.join("SKILL.md"),
                                        workspace: workspace.clone(),
                                    },
                                }],
                            },
                            result,
                        })
                        .unwrap();
                    match response
                        .await
                        .expect("steering must receive a definite outcome")
                    {
                        SteerOutcome::Rejected(error) => {
                            assert!(error.contains("Synthetic discovery failure"))
                        }
                        other => panic!("expected rejection, got {other:?}"),
                    }
                    rejected = true;
                    let (result, response) = tokio::sync::oneshot::channel();
                    controls
                        .send(SteerCommand {
                            input: SteerInput {
                                id: accepted_id,
                                prompt: "Continue".into(),
                                attachments: vec![],
                                skills: vec![],
                            },
                            result,
                        })
                        .unwrap();
                    assert!(matches!(response.await.unwrap(), SteerOutcome::Accepted));
                }
                ExecutionEvent::InputAccepted(input) => {
                    assert_eq!(input.id, accepted_id);
                    accepted = true;
                }
                ExecutionEvent::TextDelta(text) if text == "After" => continued = true,
                _ => {}
            }
        }
        task.await.unwrap().unwrap();
    })
    .await
    .unwrap();
    assert!(rejected && accepted && continued);
}

#[tokio::test]
async fn claude_steering_waits_for_replayed_uuid_and_both_result_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let script = executable(
        temp.path(),
        "steer-claude",
        r##"#!/usr/bin/env python3
import json,sys
assert '--replay-user-messages' in sys.argv
send=lambda v:print(json.dumps(v),flush=True)
initial=True
for line in sys.stdin:
 r=json.loads(line)
 if r.get('type')=='control_request':send({'type':'control_response','response':{'subtype':'success','request_id':'initialize','response':{}}})
 elif r.get('type')=='user':
  send({'type':'user','uuid':r['uuid'],'message':r['message'],'session_id':'native'})
  if initial:
   initial=False
   send({'type':'assistant','message':{'id':'before','content':[{'type':'text','text':'Before'}]}})
  else:
   assert r['message']['content'][1]['type']=='image'
   assert r['message']['content'][1]['source']['type']=='base64'
   send({'type':'result','is_error':False})
   send({'type':'assistant','message':{'id':'after','content':[{'type':'text','text':'After'}]}})
   send({'type':'result','is_error':False})
"##,
    );
    verify_steering(
        ClaudeAdapter::new(script),
        request(temp.path(), Provider::Claude),
    )
    .await;
}

async fn verify_steering(agent: impl ExecutionAdapter + 'static, request: ExecutionRequest) {
    use smaller_talk_application::{SteerCommand, SteerInput, SteerOutcome};
    let files = request.attachments.clone();
    let (events, mut receiver) = mpsc::unbounded_channel();
    let (_, responses) = mpsc::unbounded_channel();
    let (controls, control_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        agent
            .execute_controlled(request, events, responses, control_rx)
            .await
    });
    let id = Uuid::new_v4();
    let mut submitted = false;
    let mut accepted = false;
    let mut after = false;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .unwrap()
    {
        match event {
            ExecutionEvent::TextDelta(ref text) if text == "Before" && !submitted => {
                let (result, rx) = tokio::sync::oneshot::channel();
                controls
                    .send(SteerCommand {
                        input: SteerInput {
                            skills: Vec::new(),
                            id,
                            prompt: "Correction".into(),
                            attachments: files.clone(),
                        },
                        result,
                    })
                    .unwrap_or_else(|_| panic!("steering channel closed"));
                tokio::spawn(async move {
                    assert!(matches!(rx.await.unwrap(), SteerOutcome::Accepted));
                });
                submitted = true;
            }
            ExecutionEvent::InputAccepted(input) => {
                assert_eq!(input.id, id);
                accepted = true;
            }
            ExecutionEvent::TextDelta(text) if text == "After" => {
                assert!(accepted);
                after = true;
            }
            ExecutionEvent::TextSnapshot(text) if accepted => assert!(!text.contains("Before")),
            _ => {}
        }
    }
    task.await.unwrap().unwrap();
    assert!(submitted && accepted && after);
}
