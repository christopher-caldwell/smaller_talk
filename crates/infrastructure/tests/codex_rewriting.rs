#![cfg(unix)]

use smaller_talk_application::{ProviderControl, TextGenerationInput, TextGenerator};
use smaller_talk_domain::{ExecutionSettings, Provider};
use smaller_talk_infrastructure::{AgentRouter, ClaudeAdapter, CliTextGenerator, CodexAdapter};
use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration};

fn fixture(root: &Path, behavior: &str) -> (Arc<AgentRouter>, CliTextGenerator) {
    let source = format!(
        r#"#!/usr/bin/env python3
import sys,json,os,pathlib,time
if sys.argv[1:]==['exec','--help']:
 print('--ignore-user-config --ignore-rules --ephemeral --skip-git-repo-check --sandbox --json --strict-config');sys.exit(0)
args=sys.argv[1:]
assert args[0]=='exec' and args[-1]=='-'
for flag in ['--ignore-user-config','--ignore-rules','--ephemeral','--skip-git-repo-check','--strict-config','--json']:assert flag in args,flag
assert args[args.index('--sandbox')+1]=='read-only'
assert args[args.index('--model')+1]=='gpt-6-luna'
config={{}}
for i,a in enumerate(args):
 if a=='-c':
  key,value=args[i+1].split('=',1);config[key]=json.loads(value)
assert config['model_reasoning_effort']=='low'
assert config['approval_policy']=='never' and config['web_search']=='disabled'
assert config['project_doc_max_bytes']==0 and config['developer_instructions']==''
assert config['skills.include_instructions']==False and config['agents.enabled']==False
assert config['tools.update_plan.enabled']==False
assert config['tools.experimental_request_user_input.enabled']==False
for k in ['include_environment_context','include_permissions_instructions','include_apps_instructions','include_collaboration_mode_instructions']:assert config[k]==False,k
assert config.pop('features.skip_host_skill_discovery')==True
assert not any(v for k,v in config.items() if k.startswith('features.'))
for f in ['shell_tool','unified_exec','shell_snapshot','apps','plugins','hooks','multi_agent','multi_agent_v2','code_mode','code_mode_host','browser_use','computer_use','image_generation','view_image','memories','skill_search','tool_suggest','sleep_tool','goals','workspace_dependencies']:assert 'features.'+f in config,f
assert all(not k.startswith('CODEX_') or k=='CODEX_HOME' for k in os.environ)
cwd=pathlib.Path.cwd()
assert cwd.name.startswith('answer-')
assert pathlib.Path(config['model_instructions_file']).read_text()=='Synthetic instructions: café 東京'
assert json.load(sys.stdin)=={{'answer':'Synthetic answer with `src/items.rs`.'}}
def emit(v):print(json.dumps(v),flush=True)
emit({{'type':'thread.started','thread_id':'isolated'}})
emit({{'type':'turn.started'}})
{behavior}
"#
    );
    let path = root.join("codex");
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let router = AgentRouter::new(CodexAdapter::new(path), ClaudeAdapter::new("missing"));
    let policy = root.join("claude-managed-settings.json");
    std::fs::write(&policy, "{}").unwrap();
    let generator = CliTextGenerator::new(router.clone(), root.join("scratch"), vec![policy]);
    (router, generator)
}

fn input() -> TextGenerationInput {
    TextGenerationInput {
        settings: ExecutionSettings {
            provider: Provider::Codex,
            model: Some("gpt-6-luna".into()),
            effort: Some("low".into()),
            plan_mode: true, // Rewrite invocations never inherit a chat's Plan mode.
            permissions: Default::default(),
        },
        instructions: "Synthetic instructions: café 東京".into(),
        text: "Synthetic answer with `src/items.rs`.".into(),
    }
}

const SUCCESS: &str = "emit({'type':'item.completed','item':{'type':'agent_message','text':'Short answer.'}})\nemit({'type':'turn.completed','usage':{}})";

#[tokio::test]
async fn codex_rewrite_uses_isolated_controls_and_cleans_storage() {
    let dir = tempfile::tempdir().unwrap();
    let (router, generator) = fixture(dir.path(), SUCCESS);
    generator.support(Provider::Codex).await.unwrap();
    assert!(!dir.path().join("scratch").exists());
    assert_eq!(generator.generate(input()).await.unwrap(), "Short answer.");
    let mut default_effort = input();
    default_effort.settings.effort = None;
    assert_eq!(
        generator.generate(default_effort).await.unwrap(),
        "Short answer."
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("scratch"))
            .unwrap()
            .count(),
        0
    );
    // Codex support is independent of the Claude managed policy file.
    assert!(generator.support(Provider::Claude).await.is_err());
    router.configure(Provider::Codex, dir.path().join("codex"), false);
    assert!(
        generator
            .support(Provider::Codex)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
}

#[tokio::test]
async fn codex_rewrite_rejects_failed_incomplete_or_non_text_output() {
    for behavior in [
        "print('not json')",
        "emit({'type':'item.started','item':{'type':'command_execution','command':'touch forbidden'}})",
        "emit({'type':'item.completed','item':{'type':'mcp_tool_call'}})",
        "emit({'type':'item.completed','item':{'type':'web_search'}})",
        "emit({'type':'turn.failed','error':{'message':'signed out'}})",
        "emit({'type':'error','message':'quota exceeded'})",
        "emit({'type':'item.completed','item':{'type':'agent_message','text':'unfinished'}})",
        "emit({'type':'turn.completed'})",
        "emit({'type':'item.completed','item':{'type':'agent_message','text':' '}})\nemit({'type':'turn.completed'})",
        "emit({'type':'unknown.event'})",
        "sys.exit(2)",
        "print('x'*1048577)",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (_, generator) = fixture(dir.path(), behavior);
        assert!(generator.generate(input()).await.is_err(), "{behavior}");
        assert_eq!(
            std::fs::read_dir(dir.path().join("scratch"))
                .unwrap()
                .count(),
            0,
            "{behavior}"
        );
    }
}

#[tokio::test]
async fn codex_rewrite_cancellation_kills_the_child_and_cleans_storage() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let (_, generator) = fixture(
        dir.path(),
        &format!(
            "pathlib.Path({:?}).write_text(str(os.getpid()))\ntime.sleep(30)",
            pid_file.to_string_lossy()
        ),
    );
    let task = tokio::spawn(async move { generator.generate(input()).await });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !pid_file.exists() {
            assert!(
                !task.is_finished(),
                "rewrite ended before starting its child"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let pid = std::fs::read_to_string(pid_file).unwrap();
    for _ in 0..50 {
        let alive = tokio::process::Command::new("kill")
            .args(["-0", &pid])
            .output()
            .await
            .unwrap()
            .status
            .success();
        if !alive {
            assert_eq!(
                std::fs::read_dir(dir.path().join("scratch"))
                    .unwrap()
                    .count(),
                0
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("rewrite process remained alive after cancellation");
}

#[tokio::test]
async fn older_codex_is_rejected_before_starting_a_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-codex");
    std::fs::write(&path, "#!/bin/sh\necho '--json --ephemeral'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let router = AgentRouter::new(CodexAdapter::new(path), ClaudeAdapter::new("missing"));
    let generator = CliTextGenerator::new(router, dir.path().join("scratch"), vec![]);
    assert!(
        generator
            .generate(input())
            .await
            .unwrap_err()
            .to_string()
            .contains("Update Codex")
    );
    assert!(!dir.path().join("scratch").exists());
}
