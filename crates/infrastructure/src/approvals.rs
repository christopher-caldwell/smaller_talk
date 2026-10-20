use serde_json::Value;
use smaller_talk_domain::*;

pub(crate) fn present(tool: &str, input: &Value, turn_scope: bool) -> ApprovalPresentation {
    let action = if tool.to_lowercase().contains("search") {
        ApprovalAction::Search
    } else if input.get("command").is_some() || matches!(tool, "Bash" | "execCommandApproval") {
        ApprovalAction::Command
    } else if input.get("permissions").is_some() || tool.contains("permissions") {
        ApprovalAction::Permissions
    } else if input.get("file_path").is_some()
        || input.get("path").is_some()
        || tool.contains("fileChange")
        || tool.contains("Patch")
    {
        ApprovalAction::Files
    } else {
        ApprovalAction::Tool
    };
    let keys: &[(&str, &str, bool)] = match action {
        ApprovalAction::Search => &[
            ("query", "Search query", false),
            ("search_query", "Search query", false),
        ],
        ApprovalAction::Command => &[
            ("command", "Command", true),
            ("cwd", "Working folder", true),
        ],
        ApprovalAction::Files => &[
            ("file_path", "File", true),
            ("path", "File", true),
            ("changes", "Changes", true),
        ],
        ApprovalAction::Permissions => &[
            ("permissions", "Requested permissions", false),
            ("grantRoot", "Requested folder", true),
            ("networkApprovalContext", "Network access", false),
        ],
        ApprovalAction::Tool => &[],
    };
    let value_text = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string_pretty(v).unwrap_or_default())
    };
    let mut fields: Vec<_> = keys
        .iter()
        .filter_map(|(key, label, mono)| {
            input
                .get(*key)
                .filter(|v| !v.is_null())
                .map(|v| ApprovalField {
                    label: (*label).into(),
                    value: if *key == "command" {
                        v.as_array()
                            .filter(|items| items.iter().all(Value::is_string))
                            .map(|items| {
                                items
                                    .iter()
                                    .map(|item| {
                                        let part = item.as_str().expect("string arguments");
                                        if !part.is_empty()
                                            && part.chars().all(|c| {
                                                c.is_ascii_alphanumeric() || "_-./:=+".contains(c)
                                            })
                                        {
                                            part.to_owned()
                                        } else {
                                            format!("'{}'", part.replace('\'', "'\"'\"'"))
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_else(|| value_text(v))
                    } else {
                        value_text(v)
                    },
                    monospace: *mono,
                })
        })
        .collect();
    for (key, label, monospace) in [
        ("permissions", "Requested permissions", false),
        ("grantRoot", "Requested folder", true),
        ("networkApprovalContext", "Network access", false),
    ] {
        if !fields.iter().any(|f| f.label == label)
            && let Some(value) = input.get(key).filter(|v| !v.is_null())
        {
            fields.push(ApprovalField {
                label: label.into(),
                value: value_text(value),
                monospace,
            });
        }
    }
    if fields.is_empty()
        && let Some(object) = input.as_object()
    {
        fields.extend(
            object
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "reason" | "threadId" | "turnId" | "itemId"))
                .take(4)
                .map(|(key, value)| ApprovalField {
                    label: key.replace('_', " "),
                    value: value_text(value),
                    monospace: false,
                }),
        );
    }
    ApprovalPresentation {
        action,
        fields,
        explanation: input["reason"].as_str().map(str::to_owned),
        raw_arguments: serde_json::to_string_pretty(input).unwrap_or_default(),
        allow_label: if turn_scope {
            "Allow for this turn"
        } else {
            "Approve once"
        }
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_keeps_actual_newlines_and_original_arguments() {
        let input = serde_json::json!({"query":"hello\nworld", "mode":"standard"});
        let view = present("WebSearch", &input, false);
        assert_eq!(view.action, ApprovalAction::Search);
        assert_eq!(view.fields[0].value, "hello\nworld");
        assert_eq!(
            serde_json::from_str::<Value>(&view.raw_arguments).unwrap(),
            input
        );
        assert_eq!(view.allow_label, "Approve once");
    }
    #[test]
    fn array_commands_are_quoted_and_permission_context_is_visible() {
        let input = serde_json::json!({"command":["sh","-c","echo 'quoted value'"],"cwd":"/workspace","networkApprovalContext":{"host":"example.invalid"}});
        let view = present("execCommandApproval", &input, false);
        assert!(view.fields[0].value.starts_with("sh -c '"));
        assert!(view.fields.iter().any(|f| f.label == "Network access"));
        assert_eq!(
            serde_json::from_str::<Value>(&view.raw_arguments).unwrap(),
            input
        );
    }
    #[test]
    fn command_scope_and_unknown_tools_remain_visible() {
        let view = present(
            "item/permissions/requestApproval",
            &serde_json::json!({"permissions":{"network":true}}),
            true,
        );
        assert_eq!(view.allow_label, "Allow for this turn");
        let view = present("Custom", &serde_json::json!({"nested":{"a":1}}), false);
        assert_eq!(view.fields[0].label, "nested");
    }
}
