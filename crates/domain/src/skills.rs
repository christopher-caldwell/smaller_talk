use crate::Provider;
use serde::{Deserialize, Serialize};
use std::{ops::Range, path::PathBuf};

/// Provider-owned identity. Paths identify a source, not a snapshot of its contents.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillReference {
    pub provider: Provider,
    pub name: String,
    pub path: PathBuf,
    pub workspace: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillMention {
    pub range: Range<usize>,
    pub reference: SkillReference,
}
impl SkillMention {
    pub fn is_prose(&self, text: &str) -> bool {
        if text.get(self.range.clone()).is_none() {
            return false;
        }
        let probe = format!(
            "{}$probe {}",
            &text[..self.range.start],
            &text[self.range.end..]
        );
        skill_tokens(&probe)
            .iter()
            .any(|(range, _)| range.start == self.range.start)
    }

    pub fn matches(&self, text: &str) -> bool {
        text.get(self.range.clone()) == Some(format!("${}", self.reference.name).as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillDescriptor {
    pub reference: SkillReference,
    pub description: String,
    pub source: String,
    /// Native command name, which may differ from the displayed/frontmatter name.
    pub command: String,
    pub unavailable: Option<String>,
}

/// Apply a byte-range edit, dropping associations touched by it. Byte boundaries
/// come from the editor's UTF-16 conversion; no byte indexing is performed here.
pub fn edit_skill_mentions(
    mentions: &mut Vec<SkillMention>,
    range: Range<usize>,
    inserted_text: &str,
) {
    let inserted = inserted_text.len();
    mentions.retain_mut(|m| {
        // Extending a name edits the mention; adding a space or punctuation
        // after it preserves the explicitly chosen duplicate source.
        if range.is_empty()
            && range.start == m.range.end
            && inserted_text
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
        {
            return false;
        }

        if range.end <= m.range.start {
            m.range.start = m.range.start - range.len() + inserted;
            m.range.end = m.range.end - range.len() + inserted;
            true
        } else {
            range.start >= m.range.end
        }
    });
}

/// Only prose tokens participate. Escapes, quoted code, shell expansions,
/// assignments and currency stay literal. Callers resolve names against a catalog.
pub fn skill_tokens(text: &str) -> Vec<(Range<usize>, String)> {
    let mut tokens = Vec::new();
    let mut fenced = false;
    let mut offset = 0;
    let mut code_ticks = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            code_ticks = 0;
        } else if !fenced && !line.starts_with("    ") {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'`' && (i == 0 || bytes[i - 1] != b'\\') {
                    let start = i;
                    while i < bytes.len() && bytes[i] == b'`' {
                        i += 1;
                    }
                    let ticks = i - start;
                    if code_ticks == 0 {
                        code_ticks = ticks;
                    } else if code_ticks == ticks {
                        code_ticks = 0;
                    }
                    continue;
                }
                if code_ticks == 0
                    && bytes[i] == b'$'
                    && (i == 0 || bytes[i - 1].is_ascii_whitespace())
                {
                    let start = i;
                    i += 1;
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric() || b"_:-".contains(&bytes[i]))
                    {
                        i += 1;
                    }
                    let name = &line[start + 1..i];
                    if !name.is_empty()
                        && name.bytes().any(|b| b.is_ascii_lowercase())
                        && (i == bytes.len()
                            || bytes[i].is_ascii_whitespace()
                            || (b",.;!?".contains(&bytes[i])
                                && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace())))
                    {
                        tokens.push((offset + start..offset + i, name.to_owned()));
                    }
                    continue;
                }
                i += 1;
            }
        }
        offset += line.len();
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn punctuation_is_a_boundary_but_multibacktick_and_multiline_code_stays_literal() {
        let text = "$review, now ``literal $hidden code`` and `code\n$hidden` $check. ";
        assert_eq!(
            skill_tokens(text)
                .into_iter()
                .map(|(_, n)| n)
                .collect::<Vec<_>>(),
            ["review", "check"]
        );
        assert!(skill_tokens("$var/file $var.ext").is_empty());
    }
    #[test]
    fn prose_only_and_unicode_offsets() {
        let text = "🦀 $review now\n`$code` $HOME $40 ${var} $foo=1 \\$escaped\n```sh\n$hidden\n```\n$plugin:check ";
        let tokens = skill_tokens(text);
        assert_eq!(
            tokens.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(),
            ["review", "plugin:check"]
        );
        for (range, name) in tokens {
            assert_eq!(&text[range], format!("${name}"));
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::{ChatPreferences, Draft, NewChatDraft};
    fn mention(path: &str) -> SkillMention {
        SkillMention {
            range: 5..12,
            reference: SkillReference {
                provider: Provider::Codex,
                name: "review".into(),
                path: path.into(),
                workspace: "/work".into(),
            },
        }
    }
    #[test]
    fn edits_shift_utf8_offsets_and_remove_touched_identity() {
        let mut skills = vec![mention("/one/SKILL.md")];
        edit_skill_mentions(&mut skills, 0..0, "🦀");
        assert_eq!(skills[0].range, 9..16);
        edit_skill_mentions(&mut skills, 16..16, " ");
        assert_eq!(skills[0].range, 9..16);
        edit_skill_mentions(&mut skills, 11..12, "");
        assert!(skills.is_empty());
    }
    #[test]
    fn literalizing_an_explicit_mention_removes_prose_eligibility() {
        let mut selected = mention("/chosen/SKILL.md");
        assert!(selected.is_prose("test $review"));
        selected.range = 6..13;
        assert!(!selected.is_prose("test `$review`"));
        assert!(!selected.is_prose("test x$review"));
    }
    #[test]
    fn extending_name_removes_identity_but_delimiter_preserves_duplicate_source() {
        let original = mention("/selected-duplicate/SKILL.md");
        let mut skills = vec![original.clone()];
        edit_skill_mentions(&mut skills, 12..12, " ");
        assert_eq!(skills, [original]);
        edit_skill_mentions(&mut skills, 12..12, "-other");
        assert!(skills.is_empty());
    }
    #[test]
    fn acknowledging_same_text_different_source_preserves_draft() {
        let draft = Draft {
            text: "test $review".into(),
            skills: vec![mention("/two/SKILL.md")],
            ..Default::default()
        };
        let mut submitted = draft.clone();
        submitted.skills[0].reference.path = "/one/SKILL.md".into();
        let pending = NewChatDraft {
            id: "new".into(),
            preferences: ChatPreferences::default(),
            draft: draft.clone(),
            submitted: Some(submitted),
        };
        assert_eq!(pending.remaining_draft(), draft);
    }
    #[test]
    fn legacy_draft_loads_without_skills() {
        let draft: Draft = serde_json::from_str(
            r#"{"text":"old","attachments":[],"selection_start":0,"selection_end":0}"#,
        )
        .unwrap();
        assert!(draft.skills.is_empty());
    }
}
