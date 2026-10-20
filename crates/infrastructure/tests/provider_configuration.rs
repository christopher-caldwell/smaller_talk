#![cfg(unix)]
use smaller_talk_application::*;
use smaller_talk_domain::*;
use smaller_talk_infrastructure::*;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn script(root: &Path, name: &str, text: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
#[tokio::test]
async fn provider_readiness_does_not_start_a_session_or_infer_access_from_models() {
    let dir = tempfile::tempdir().unwrap();
    let claude = script(
        dir.path(),
        "claude",
        "#!/usr/bin/env python3\nimport sys,json\nassert sys.argv[1:]==['auth','status','--json']\nprint(json.dumps({'loggedIn':False}))\n",
    );
    let codex = dir.path().join("missing");
    let router = AgentRouter::new(CodexAdapter::new(&codex), ClaudeAdapter::new(&claude));
    assert_eq!(
        router.readiness(Provider::Codex).await,
        ProviderReadiness::Missing
    );
    assert_eq!(
        router.readiness(Provider::Claude).await,
        ProviderReadiness::SignedOut
    );
    router.configure(Provider::Claude, claude, false);
    assert!(
        router
            .capabilities(Provider::Claude)
            .await
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
}
#[tokio::test]
async fn catalogs_refresh_and_ignore_results_from_replaced_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("discoveries");
    let source = format!(
        r#"#!/usr/bin/env python3
import sys,json,time
for line in sys.stdin:
 r=json.loads(line)
 if r.get('method')=='initialize': print(json.dumps({{'id':r['id'],'result':{{}}}}),flush=True)
 if r.get('method')=='model/list':
  with open({marker:?},'a') as f:f.write('x\n')
  time.sleep(.15)
  print(json.dumps({{'id':r['id'],'result':{{'data':[{{'model':'fixture','displayName':'Fixture','isDefault':True}}],'nextCursor':None}}}}),flush=True)
"#,
        marker = marker.to_string_lossy()
    );
    let codex = script(dir.path(), "codex", &source);
    let router = AgentRouter::new(CodexAdapter::new(&codex), ClaudeAdapter::new("missing"));
    let in_flight = router.clone();
    let task = tokio::spawn(async move { in_flight.capabilities(Provider::Codex).await });
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    router.configure(Provider::Codex, codex.clone(), false);
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("configuration changed")
    );
    router.configure(Provider::Codex, codex, true);
    router.capabilities(Provider::Codex).await.unwrap();
    router.capabilities(Provider::Codex).await.unwrap();
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);
    router.refresh(Provider::Codex).await.unwrap();
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 3);
}
fn generator(dir: &Path, tools: &str) -> CliTextGenerator {
    let source = format!(
        r#"#!/usr/bin/env python3
import json,sys,os
if '--help' in sys.argv:
 print('--safe-mode --tools --strict-mcp-config --disable-slash-commands --no-session-persistence --setting-sources --system-prompt');sys.exit(0)
assert '--safe-mode' in sys.argv and '--no-session-persistence' in sys.argv and '--disable-slash-commands' in sys.argv
assert sys.argv[sys.argv.index('--tools')+1]==''
assert sys.argv[sys.argv.index('--setting-sources')+1]==''
assert '--strict-mcp-config' in sys.argv
assert os.path.basename(os.getcwd()).startswith('answer-')
input=json.load(sys.stdin);assert input=={{'answer':'synthetic answer'}}
print(json.dumps({{'type':'system','subtype':'init','tools':{tools},'mcp_servers':[]}}))
print(json.dumps({{'type':'result','is_error':False,'result':'short result'}}))
"#
    );
    let path = script(dir, "claude", &source);
    let router = AgentRouter::new(CodexAdapter::new("missing"), ClaudeAdapter::new(path));
    CliTextGenerator::new(router, dir.join("scratch"), vec![])
}
fn input() -> TextGenerationInput {
    TextGenerationInput {
        settings: ExecutionSettings {
            provider: Provider::Claude,
            model: Some("fixture".into()),
            ..Default::default()
        },
        instructions: "synthetic instructions".into(),
        text: "synthetic answer".into(),
    }
}
#[tokio::test]
async fn isolated_generation_requires_an_empty_tool_catalog_and_cleans_scratch() {
    let dir = tempfile::tempdir().unwrap();
    let generator = generator(dir.path(), "[]");
    assert_eq!(generator.generate(input()).await.unwrap(), "short result");
    assert_eq!(
        std::fs::read_dir(dir.path().join("scratch"))
            .unwrap()
            .count(),
        0
    );
    assert!(generator.support(Provider::Codex).await.is_err());
    let dir = tempfile::tempdir().unwrap();
    assert!(
        self::generator(dir.path(), "['Bash']")
            .generate(input())
            .await
            .unwrap_err()
            .to_string()
            .contains("empty tool")
    );
}
#[tokio::test]
async fn managed_policy_disables_rewriting_before_a_model_is_called() {
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("managed-settings.json");
    std::fs::write(&policy, "{}").unwrap();
    let router = AgentRouter::new(CodexAdapter::new("missing"), ClaudeAdapter::new("missing"));
    let generator = CliTextGenerator::new(router, dir.path().join("scratch"), vec![policy]);
    assert!(
        generator
            .support(Provider::Claude)
            .await
            .unwrap_err()
            .to_string()
            .contains("managed")
    );
    assert!(!dir.path().join("scratch").exists());
}
