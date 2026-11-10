//! Slash commands in the assistant prompt (`/` at character index 0 only).

/// One slash command shown in the popup /help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
}

/// Catalog in display order.
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "new",
        usage: "/new",
        summary: "Start a new chat (also Ctrl+L)",
    },
    SlashCommand {
        name: "clear",
        usage: "/clear",
        summary: "Same as /new",
    },
    SlashCommand {
        name: "help",
        usage: "/help",
        summary: "List slash commands and key hints",
    },
    SlashCommand {
        name: "plan",
        usage: "/plan",
        summary: "Plan mode: read code and write a plan, no edits",
    },
    SlashCommand {
        name: "ask",
        usage: "/ask",
        summary: "Ask mode: read code and answer, no edits",
    },
    SlashCommand {
        name: "agent",
        usage: "/agent",
        summary: "Agent mode: read and edit files (default)",
    },
    SlashCommand {
        name: "changes",
        usage: "/changes [n]",
        summary: "List the assistant's edits; n opens that one",
    },
    SlashCommand {
        name: "model",
        usage: "/model [name]",
        summary: "Show or switch the model",
    },
    SlashCommand {
        name: "provider",
        usage: "/provider [ollama|lmstudio|openai-compatible|openrouter]",
        summary: "Show or switch the AI provider",
    },
    SlashCommand {
        name: "context",
        usage: "/context [n]",
        summary: "Show or set context size (num_ctx)",
    },
    SlashCommand {
        name: "stream",
        usage: "/stream on|off",
        summary: "Toggle response streaming",
    },
    SlashCommand {
        name: "auto-approve",
        usage: "/auto-approve on|off",
        summary: "Toggle tool auto-approve",
    },
    SlashCommand {
        name: "think",
        usage: "/think on|off",
        summary: "Toggle sending think:true to Ollama",
    },
    SlashCommand {
        name: "stop",
        usage: "/stop",
        summary: "Stop the assistant. The only way to stop it",
    },
    SlashCommand {
        name: "retry",
        usage: "/retry",
        summary: "Resend the last user prompt",
    },
    SlashCommand {
        name: "compact",
        usage: "/compact",
        summary: "Drop older history to free context",
    },
    SlashCommand {
        name: "settings",
        usage: "/settings",
        summary: "Open Settings → AI",
    },
];

/// Parsed `/name args` when the prompt is a slash command at position 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSlash {
    pub name: String,
    pub args: String,
}

/// Only when the first character is `/` (no leading whitespace).
pub fn parse_slash(prompt: &str) -> Option<ParsedSlash> {
    let mut chars = prompt.chars();
    if chars.next() != Some('/') {
        return None;
    }
    let rest = chars.as_str();
    let (name, args) = match rest.find(char::is_whitespace) {
        Some(at) => {
            let name = &rest[..at];
            let args = rest[at..].trim_start();
            (name, args)
        }
        None => (rest, ""),
    };
    if name.is_empty() {
        return Some(ParsedSlash {
            name: String::new(),
            args: String::new(),
        });
    }
    // Reject path-like tokens: /usr/bin, //, or name containing '/'
    if name.contains('/') {
        return None;
    }
    Some(ParsedSlash {
        name: name.to_ascii_lowercase(),
        args: args.to_string(),
    })
}

/// True when the prompt should be treated as a (possibly incomplete) slash command.
pub fn is_slash_prompt(prompt: &str) -> bool {
    prompt.starts_with('/')
}

/// Filter text after `/` up to the first whitespace (for the popup).
pub fn slash_filter(prompt: &str) -> Option<&str> {
    if !prompt.starts_with('/') {
        return None;
    }
    let rest = &prompt[1..];
    let end = rest
        .find(char::is_whitespace)
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Fuzzy-ish prefix filter: case-insensitive; every filter char must appear in order.
pub fn filter_commands(filter: &str) -> Vec<&'static SlashCommand> {
    let needle: Vec<char> = filter.to_ascii_lowercase().chars().collect();
    let mut out: Vec<&'static SlashCommand> = COMMANDS
        .iter()
        .filter(|cmd| matches_fuzzy(&cmd.name.to_ascii_lowercase(), &needle))
        .collect();
    // Prefer exact / prefix matches first, then shorter names, then catalog order.
    let place = |cmd: &SlashCommand| COMMANDS.iter().position(|c| c.name == cmd.name);
    out.sort_by(|a, b| {
        let a_pref = a.name.starts_with(&filter.to_ascii_lowercase()) as i32;
        let b_pref = b.name.starts_with(&filter.to_ascii_lowercase()) as i32;
        b_pref
            .cmp(&a_pref)
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| place(a).cmp(&place(b)))
    });
    out
}

fn matches_fuzzy(hay: &str, needle: &[char]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut it = hay.chars();
    for want in needle {
        loop {
            match it.next() {
                Some(ch) if ch == *want => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// Resolve a parsed name to a known command, if any.
pub fn lookup(name: &str) -> Option<&'static SlashCommand> {
    let lower = name.to_ascii_lowercase();
    COMMANDS.iter().find(|cmd| cmd.name == lower)
}

pub fn help_text() -> String {
    let mut lines = vec![
        "Slash commands (must start at the first character):".to_string(),
        String::new(),
    ];
    for cmd in COMMANDS {
        lines.push(format!("  {:<22} {}", cmd.usage, cmd.summary));
    }
    lines.push(String::new());
    lines.push("Keys: Enter send · Shift+Enter newline · Ctrl+L new chat · Alt+Up/Down panel height · Esc hides the panel".into());
    lines.push("A running turn keeps going when the panel hides. /stop is the only way to stop it.".into());
    lines.push("@file: type @ after a space (or at start) to attach a workspace file.".into());
    lines.push(
        "Approval: [ Yes ] [ Yes, this session ] [ No ] · ←/→ Tab h/l move · y yes · a this session · n/Esc no"
            .into(),
    );
    lines.push(
        "  \"This session\" allows that tool (create_file / apply_edit / save) until /new, /clear or Ctrl+L. Command tools always ask."
            .into(),
    );
    lines.push(
        "Modes: /plan and /ask only read files; /agent edits again. The title reads \"ready : plan\". The editor itself has no modes."
            .into(),
    );
    lines.push(
        "Edits stay unsaved in the editor. Ctrl+Z undoes one. /changes lists them, /changes 2 opens the second."
            .into(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_mentions_session_approval() {
        let help = help_text();
        assert!(help.contains("Yes, this session"), "{help}");
        assert!(help.contains("a this session"), "{help}");
    }

    #[test]
    fn only_position_zero_is_a_command() {
        assert!(parse_slash("/help").is_some());
        assert!(parse_slash("/model qwen").is_some());
        assert!(parse_slash(" /help").is_none());
        assert!(parse_slash("please /help me").is_none());
        assert!(parse_slash("see /usr/bin").is_none());
        assert_eq!(parse_slash("/usr/bin"), None);
        assert_eq!(parse_slash("//oops"), None);
    }

    #[test]
    fn parses_name_and_args() {
        let parsed = parse_slash("/model qwen3:0.6b").unwrap();
        assert_eq!(parsed.name, "model");
        assert_eq!(parsed.args, "qwen3:0.6b");
        let parsed = parse_slash("/NEW").unwrap();
        assert_eq!(parsed.name, "new");
        assert!(parsed.args.is_empty());
    }

    #[test]
    fn filter_prefers_prefix() {
        let hits = filter_commands("mo");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name, "model");
        let hits = filter_commands("hel");
        assert_eq!(hits[0].name, "help");
        let hits = filter_commands("zzz");
        assert!(hits.is_empty());
    }

    #[test]
    fn unknown_name_still_parses() {
        let parsed = parse_slash("/xyz").unwrap();
        assert_eq!(parsed.name, "xyz");
        assert!(lookup("xyz").is_none());
    }
}
