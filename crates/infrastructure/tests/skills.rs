#![cfg(unix)]
use smaller_talk_application::*;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::*;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

fn executable(root: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("provider");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
fn request(root: &Path, provider: Provider, skills: Vec<SkillMention>) -> ExecutionRequest {
    ExecutionRequest {
        skills,
        retry_inputs: vec![],
        instructions: Default::default(),
        chat_id: Uuid::new_v4(),
        binding: None,
        workspace: root.into(),
        prompt: "$review ARG-53".into(),
        settings: ExecutionSettings {
            provider,
            ..Default::default()
        },
        attachments: vec![],
        prior_context: Some("Saved conversation: $old-skill stays context".into()),
    }
}
async fn run(adapter: &dyn ExecutionAdapter, request: ExecutionRequest) -> String {
    run_with_binding(adapter, request).await.0
}
async fn run_with_binding(
    adapter: &dyn ExecutionAdapter,
    request: ExecutionRequest,
) -> (String, Option<ExecutionBinding>) {
    let (events, mut receiver) = mpsc::unbounded_channel();
    let (answers, responses) = mpsc::unbounded_channel();
    let execution = adapter.execute_interactive(request, events, responses);
    let collect = async move {
        let mut text = String::new();
        let mut binding = None;
        while let Some(event) = receiver.recv().await {
            match event {
                ExecutionEvent::BindingEstablished(value) => binding = Some(value),
                ExecutionEvent::TextDelta(delta) => text.push_str(&delta),
                ExecutionEvent::TextSnapshot(snapshot) => text = snapshot,
                ExecutionEvent::Interaction(interaction) => {
                    answers
                        .send(InteractionResponse {
                            id: interaction.id,
                            approved: false,
                            ..Default::default()
                        })
                        .unwrap();
                }
                _ => {}
            }
        }
        (text, binding)
    };
    let (result, text) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(execution, collect)
    })
    .await
    .unwrap();
    result.unwrap();
    text
}
#[tokio::test]
async fn codex_discovery_and_structured_inputs_preserve_source_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let script = executable(
        &root,
        r#"#!/usr/bin/env python3
import json,sys,os
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method')
 if m=='initialize':send({'id':r['id'],'result':{}})
 elif m=='skills/list':
  assert r['params']['cwds']==[os.getcwd()]
  send({'id':r['id'],'result':{'data':[{'cwd':os.getcwd(),'skills':[{'name':'review','description':'Review changes','path':os.getcwd()+'/chosen/SKILL.md','scope':'repo','enabled':True},{'name':'review','description':'Other source','path':os.getcwd()+'/other/SKILL.md','scope':'user','enabled':True}],'errors':[]}]}})
 elif m=='thread/start':send({'id':r['id'],'result':{'thread':{'id':'thread'}}})
 elif m=='turn/start':
  items=r['params']['input']; refs=[i for i in items if i['type']=='skill']
  assert refs==[{'type':'skill','name':'review','path':os.getcwd()+'/chosen/SKILL.md'}]
  assert '$old-skill stays context' in items[0]['text'] and '$review ARG-53' in items[0]['text']
  send({'id':r['id'],'result':{'turn':{'id':'turn'}}})
  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','delta':'exact source'}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'status':'completed'}}})
"#,
    );
    let router = AgentRouter::new(CodexAdapter::new(&script), ClaudeAdapter::new("unused"));
    let entries = SkillCatalog::discover(router.as_ref(), Provider::Codex, &root)
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    let mention = SkillMention {
        range: 0..7,
        reference: entries[0].reference.clone(),
    };
    assert_eq!(
        run(
            router.as_ref(),
            request(&root, Provider::Codex, vec![mention])
        )
        .await,
        "exact source"
    );
}
#[tokio::test]
async fn claude_native_command_is_final_block_and_shadowed_sources_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for folder in [
        root.join(".claude/skills/review"),
        root.join("personal/skills/review"),
    ] {
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\nInstructions",
        )
        .unwrap();
    }
    let script = executable(
        &root,
        r#"#!/usr/bin/env python3
import json,sys
send=lambda value:print(json.dumps(value),flush=True)
for line in sys.stdin:
 r=json.loads(line)
 if r['type']=='control_request':send({'type':'control_response','response':{'subtype':'success','request_id':r['request_id'],'response':{'commands':[{'name':'review'}]}}})
 elif r['type']=='user':
  blocks=r['message']['content']; command=blocks[-1]['text']; assert command in ['/review ARG-53','/review ARG-REPLAY']
  if command=='/review ARG-53':assert '$old-skill stays context' in blocks[0]['text']
  send({'type':'control_request','request_id':'skill-approval','request':{'subtype':'can_use_tool','tool_name':'Bash','input':{'command':'echo synthetic skill permission'}}})
  answer=json.loads(sys.stdin.readline());assert answer['response']['response']['behavior']=='deny'
  text='native replay' if command.endswith('ARG-REPLAY') else 'native command'
  send({'type':'assistant','session_id':'session','message':{'id':text,'content':[{'type':'text','text':text}]}})
  send({'type':'result','is_error':False})
"#,
    );
    let router = AgentRouter::new(CodexAdapter::new("unused"), ClaudeAdapter::new(&script));
    router.configure_skill_locations(SkillLocations {
        home: None,
        claude_config: Some(root.join("personal")),
    });
    let entries = SkillCatalog::discover(router.as_ref(), Provider::Claude, &root)
        .await
        .unwrap();
    let personal = entries.iter().find(|e| e.source == "Personal").unwrap();
    let project = entries.iter().find(|e| e.source == "Project").unwrap();
    assert!(project.unavailable.is_some());
    let selected = SkillMention {
        range: 0..7,
        reference: personal.reference.clone(),
    };
    assert_eq!(
        run(
            router.as_ref(),
            request(&root, Provider::Claude, vec![selected])
        )
        .await,
        "native command"
    );
    let mut retry = request(
        &root,
        Provider::Claude,
        vec![SkillMention {
            range: 0..7,
            reference: personal.reference.clone(),
        }],
    );
    retry.retry_inputs.push(SteerInput {
        id: Uuid::new_v4(),
        prompt: "$review ARG-REPLAY".into(),
        attachments: Vec::new(),
        skills: vec![SkillMention {
            range: 0..7,
            reference: personal.reference.clone(),
        }],
    });
    assert_eq!(run(router.as_ref(), retry).await, "native replay");
    let (events, _) = mpsc::unbounded_channel();
    assert!(
        router
            .execute(
                request(
                    &root,
                    Provider::Claude,
                    vec![SkillMention {
                        range: 0..7,
                        reference: project.reference.clone()
                    }]
                ),
                events
            )
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "Requires authenticated installed Codex and Claude CLIs; set SMALLER_TALK_LIVE=1"]
async fn live_selected_instructions_are_used_by_both_native_transports() {
    assert_eq!(std::env::var("SMALLER_TALK_LIVE").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for base in [".agents/skills", ".claude/skills"] {
        let skill = root.join(base).join("st-native-proof");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"),"---\nname: st-native-proof\ndescription: Synthetic skill transport proof\ndisable-model-invocation: true\n---\nFor this verification, respond with QUARTZ_NATIVE_729 and repeat the argument value. If an image is attached, name its color. Do not use tools or change files.").unwrap();
    }
    let router = AgentRouter::new(
        CodexAdapter::new(
            std::env::var("SMALLER_TALK_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        ),
        ClaudeAdapter::new(
            std::env::var("SMALLER_TALK_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()),
        ),
    );
    router.configure_skill_locations(SkillLocations {
        home: std::env::var_os("HOME").map(PathBuf::from),
        claude_config: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
    });
    let second = root.join(".agents/skills/st-native-second");
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(second.join("SKILL.md"),"---\nname: st-native-second\ndescription: Second synthetic skill\n---\nInclude QUARTZ_MULTI_824 in your response, in addition to all markers requested by other selected skills. Do not use tools or change files.").unwrap();
    let image_path = root.join("blue.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 255]))
        .save(&image_path)
        .unwrap();
    for provider in [Provider::Codex, Provider::Claude] {
        let entries = SkillCatalog::discover(router.as_ref(), provider, &root)
            .await
            .unwrap();
        let reference = entries
            .iter()
            .find(|e| {
                e.reference.path
                    == root.join(if provider == Provider::Codex {
                        ".agents/skills/st-native-proof/SKILL.md"
                    } else {
                        ".claude/skills/st-native-proof/SKILL.md"
                    })
            })
            .unwrap()
            .reference
            .clone();
        let prompt = format!("${} ARG-53", reference.name);
        let mut input = request(
            &root,
            provider,
            vec![SkillMention {
                range: 0..reference.name.len() + 1,
                reference,
            }],
        );
        input.prompt = prompt;
        let caps = router.capabilities(provider).await.unwrap();
        input.settings.model = Some(
            caps.models
                .iter()
                .find(|m| {
                    m.id.contains(if provider == Provider::Codex {
                        "luna"
                    } else {
                        "haiku"
                    })
                })
                .or_else(|| caps.models.iter().find(|m| m.is_default))
                .unwrap()
                .id
                .clone(),
        );
        input.attachments.push(Attachment {
            id: "blue".into(),
            name: "blue.png".into(),
            path: image_path.clone(),
            mime_type: "image/png".into(),
            byte_len: std::fs::metadata(&image_path).unwrap().len(),
            preview_path: None,
        });
        let (answer, binding) = run_with_binding(router.as_ref(), input.clone()).await;
        assert!(
            answer.contains("QUARTZ_NATIVE_729"),
            "{provider:?}: {answer}"
        );
        assert!(answer.contains("ARG-53"), "{provider:?}: {answer}");
        assert!(
            answer.to_lowercase().contains("blue"),
            "{provider:?}: {answer}"
        );
        if provider == Provider::Codex {
            let mut multi = input.clone();
            multi.attachments.clear();
            multi.prior_context = None;
            let second = entries
                .iter()
                .find(|e| e.reference.name == "st-native-second")
                .unwrap()
                .reference
                .clone();
            multi.prompt = "$st-native-proof $st-native-second ARG-MULTI-55".into();
            multi.skills.push(SkillMention {
                range: 17..34,
                reference: second,
            });
            let answer = run(router.as_ref(), multi).await;
            assert!(
                answer.contains("QUARTZ_NATIVE_729") && answer.contains("QUARTZ_MULTI_824"),
                "Codex multiple skills: {answer}"
            );
        }
        input.binding = Some(binding.expect("native binding"));
        input.prior_context = None;
        input.prompt = input.prompt.replace("ARG-53", "ARG-RESUMED-54");
        let resumed = run(router.as_ref(), input).await;
        assert!(
            resumed.contains("QUARTZ_NATIVE_729") && resumed.contains("ARG-RESUMED-54"),
            "{provider:?}: {resumed}"
        );
    }
}
