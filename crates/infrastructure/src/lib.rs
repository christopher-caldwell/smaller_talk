mod attachments;
mod catalog;
mod claude;
mod codex;

pub use attachments::LocalAttachments;
pub use catalog::AgentRouter;
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;

mod steering;

mod approvals;

mod repository;
pub use repository::GitRepositoryReader;

mod rewriting;
pub use rewriting::CliTextGenerator;

mod skills;
pub use skills::SkillLocations;
