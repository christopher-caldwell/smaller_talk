use serde::{Deserialize, Serialize};

pub const MAX_INSTRUCTION_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomInstructions {
    pub enabled: bool,
    pub text: String,
}

impl CustomInstructions {
    pub fn is_valid(&self) -> bool {
        self.text.len() <= MAX_INSTRUCTION_BYTES && !self.text.contains('\0')
    }

    pub fn active_text(&self) -> Option<&str> {
        (self.enabled && !self.text.trim().is_empty()).then_some(self.text.as_str())
    }
}

/// Version zero identifies a turn recorded before app instructions were available.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionSnapshot {
    pub version: u32,
    pub global: String,
    pub project: String,
    pub project_id: Option<String>,
    pub text: String,
    pub fingerprint: Option<String>,
}
