//! `@file` mentions in the assistant prompt.

use crate::picker::{self, IndexedFile};

/// Max files shown in the @ popup.
pub const MENTION_LIMIT: usize = 10;

/// Budget for attached file bodies sent with one prompt (characters).
pub const ATTACH_BUDGET: usize = 12_000;
/// Soft per-file cap inside the attach budget.
pub const ATTACH_FILE_CAP: usize = 6_000;

/// Active `@query` under the cursor (if any).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMention {
    /// Char index of `@`.
    pub at: usize,
    /// Text after `@` up to the cursor (unquoted).
    pub query: String,
    /// Char index just after the query (usually the cursor).
    pub end: usize,
}

/// One `@path` span in the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionSpan {
    pub start: usize,
    pub end: usize,
    pub path: String,
}

/// `@` opens a mention when it is at the start or after whitespace,
/// and not part of an email like `a@b.com`.
pub fn mention_trigger_at(text: &str, at_char: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if at_char >= chars.len() || chars[at_char] != '@' {
        return false;
    }
    if at_char == 0 {
        return true;
    }
    let prev = chars[at_char - 1];
    if prev.is_whitespace() {
        return true;
    }
    // email-like: alnum or . _ - before @ → not a trigger
    if prev.is_alphanumeric() || prev == '.' || prev == '_' || prev == '-' {
        return false;
    }
    false
}

/// Find the active mention token containing `cursor` (char index).
pub fn active_mention(prompt: &str, cursor: usize) -> Option<ActiveMention> {
    let chars: Vec<char> = prompt.chars().collect();
    let cursor = cursor.min(chars.len());
    // Walk left from cursor-1 to find `@` start of token.
    let mut i = cursor;
    while i > 0 {
        i -= 1;
        if chars[i] == '@' {
            if !mention_trigger_at(prompt, i) {
                return None;
            }
            // If cursor is before/at @, still opening.
            if cursor <= i {
                return None;
            }
            // Scan the token after `@` to know when it already ended (whitespace).
            let mut end = i + 1;
            let mut in_quote = false;
            while end < chars.len() {
                let ch = chars[end];
                if ch == '"' {
                    in_quote = !in_quote;
                    end += 1;
                    continue;
                }
                if !in_quote && ch.is_whitespace() {
                    break;
                }
                end += 1;
            }
            // Finished token and cursor is past it → no popup.
            if cursor > end && end < chars.len() && chars[end].is_whitespace() {
                return None;
            }
            let end = cursor.max(i + 1).min(chars.len());
            let query: String = chars[i + 1..end].iter().collect();
            let query = strip_quotes(&query);
            return Some(ActiveMention {
                at: i,
                query,
                end,
            });
        }
        if chars[i].is_whitespace() {
            break;
        }
    }
    None
}

fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].to_string()
    } else {
        t.trim_matches('"').to_string()
    }
}

/// Parse all complete `@path` mentions in a prompt (for send).
pub fn parse_mentions(prompt: &str) -> Vec<MentionSpan> {
    let chars: Vec<char> = prompt.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && mention_trigger_at(prompt, i) {
            let start = i;
            i += 1;
            let mut path = String::new();
            if i < chars.len() && chars[i] == '"' {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    path.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    i += 1;
                }
            } else {
                while i < chars.len() && !chars[i].is_whitespace() {
                    path.push(chars[i]);
                    i += 1;
                }
            }
            if !path.is_empty() {
                out.push(MentionSpan {
                    start,
                    end: i,
                    path,
                });
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Insert `@relative` replacing the active mention token. Quotes when needed.
pub fn insert_mention(prompt: &str, active: &ActiveMention, relative: &str) -> (String, usize) {
    let chars: Vec<char> = prompt.chars().collect();
    let token = format_mention_token(relative);
    let token_chars: Vec<char> = token.chars().collect();
    // Replace from `@` through current query end (cursor region).
    let mut end = active.end;
    // Extend end to consume the rest of an unfinished token (no whitespace yet).
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&chars[..active.at]);
    out.extend_from_slice(&token_chars);
    // Prefer a trailing space after insert for continued typing.
    let need_space = end >= chars.len() || !chars[end].is_whitespace();
    if need_space {
        out.push(' ');
    }
    out.extend_from_slice(&chars[end..]);
    let cursor = active.at + token_chars.len() + usize::from(need_space);
    (out.into_iter().collect(), cursor)
}

pub fn format_mention_token(relative: &str) -> String {
    if relative.chars().any(|c| c.is_whitespace()) {
        format!("@\"{relative}\"")
    } else {
        format!("@{relative}")
    }
}

/// Rank workspace files for the @ popup (reuse picker scorer).
pub fn suggest_files(
    files: &[IndexedFile],
    query: &str,
    recent: &[String],
    active_dir: &str,
) -> Vec<u32> {
    let ranked = picker::filter(files, query, recent, active_dir);
    ranked.into_iter().take(MENTION_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_does_not_trigger() {
        assert!(!mention_trigger_at("a@b.com", 1));
        assert!(!mention_trigger_at("mail test@example.com", 9));
        assert!(mention_trigger_at("@file", 0));
        assert!(mention_trigger_at("see @file", 4));
        assert!(mention_trigger_at("ดู @ไฟล์", 3));
    }

    #[test]
    fn active_mention_reads_query() {
        let m = active_mention("@mai", 4).unwrap();
        assert_eq!(m.at, 0);
        assert_eq!(m.query, "mai");
        let m = active_mention("check @src/a", 12).unwrap();
        assert_eq!(m.query, "src/a");
        assert!(active_mention("a@b.com", 3).is_none());
        assert!(active_mention("hello world", 5).is_none());
    }

    #[test]
    fn parse_mentions_supports_quotes_and_unicode() {
        let spans = parse_mentions("look @src/a.rs and @\"my file.rs\" then @ไฟล์/ทดสอบ.rs");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].path, "src/a.rs");
        assert_eq!(spans[1].path, "my file.rs");
        assert_eq!(spans[2].path, "ไฟล์/ทดสอบ.rs");
    }

    #[test]
    fn insert_quotes_paths_with_spaces() {
        let active = ActiveMention {
            at: 0,
            query: "my".into(),
            end: 3,
        };
        let (out, cursor) = insert_mention("@my", &active, "my file.rs");
        assert!(out.starts_with("@\"my file.rs\""));
        assert!(cursor > 0);
    }

    #[test]
    fn path_mid_prompt_is_not_slash_but_at_works() {
        // sanity with slash module boundary
        assert!(crate::agent::slash::parse_slash("see /usr/bin").is_none());
        assert!(mention_trigger_at("see @usr", 4));
    }

    #[test]
    fn attach_budget_constants() {
        const { assert!(ATTACH_BUDGET >= ATTACH_FILE_CAP) };
        const { assert!(ATTACH_FILE_CAP > 0) };
        assert_eq!(MENTION_LIMIT, 10);
    }

    fn indexed(path: &str) -> IndexedFile {
        IndexedFile {
            name_at: path.rfind('/').map(|i| i as u32 + 1).unwrap_or(0),
            relative: path.to_string(),
        }
    }

    #[test]
    fn suggest_files_ranks_basename_and_caps() {
        let files = vec![
            indexed("src/main.rs"),
            indexed("docs/main.md"),
            indexed("src/lib.rs"),
        ];
        let recent = vec!["src/main.rs".to_string()];
        let rows = suggest_files(&files, "main", &recent, "src");
        assert!(!rows.is_empty());
        assert!(rows.len() <= MENTION_LIMIT);
        // recent + active dir should prefer src/main.rs
        assert_eq!(files[rows[0] as usize].relative, "src/main.rs");
    }
}
