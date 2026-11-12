//! The last few tool calls of one tab as a ticker line, free of host calls so it tests natively.

use std::collections::VecDeque;

/// Untrusted text on one line, without control or format characters, and at most `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control() && !is_format(*c)).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max { one } else { format!("{}…", one.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

fn is_format(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{115f}' | '\u{1160}' | '\u{17b4}' | '\u{17b5}' | '\u{180b}'..='\u{180f}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}' | '\u{ffa0}' | '\u{fff0}'..='\u{fffb}' | '\u{e0000}'..='\u{e0fff}')
}

/// One step: `Read session.ts`, `Bash npm test`, `Grep "aws_iam"`.
pub fn step(tool: &str, file_path: Option<&str>, command: Option<&str>, pattern: Option<&str>, arg_max: usize) -> String {
    let tool = clean(tool, 24);
    let arg = if let Some(p) = file_path.filter(|p| !p.is_empty()) {
        clean(p.trim_end_matches('/').rsplit('/').next().unwrap_or(p), arg_max)
    } else if let Some(c) = command.filter(|c| !c.trim().is_empty()) {
        clean(c, arg_max)
    } else if let Some(p) = pattern.filter(|p| !p.is_empty()) {
        format!("\"{}\"", clean(p, arg_max))
    } else {
        String::new()
    };
    if arg.is_empty() { tool } else { format!("{tool} {arg}") }
}

#[derive(Debug, Clone, Default)]
pub struct Ticker {
    steps: VecDeque<String>,
}

impl Ticker {
    pub fn push(&mut self, step: String, keep: usize) {
        if step.is_empty() {
            return;
        }
        self.steps.push_back(step);
        while self.steps.len() > keep.max(1) {
            self.steps.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.steps.clear();
    }

    /// Oldest first, each followed by an arrow, cut from the front to `max` characters.
    pub fn line(&self, max: usize) -> Option<String> {
        if self.steps.is_empty() {
            return None;
        }
        let mut parts: Vec<&str> = self.steps.iter().map(String::as_str).collect();
        loop {
            let line = parts.iter().map(|s| format!("{s} →")).collect::<Vec<_>>().join(" ");
            if line.chars().count() <= max || parts.len() == 1 {
                return Some(if line.chars().count() <= max { line } else { clean(&line, max) });
            }
            parts.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_name_the_file_command_or_pattern() {
        assert_eq!(step("Read", Some("/w/web/src/session.ts"), None, None, 32), "Read session.ts");
        assert_eq!(step("Bash", None, Some("npm  test\n"), None, 32), "Bash npm test");
        assert_eq!(step("Grep", None, None, Some("aws_iam"), 32), "Grep \"aws_iam\"");
        assert_eq!(step("TodoWrite", None, Some(" "), None, 32), "TodoWrite");
        assert_eq!(step("Bash", None, Some("echo \u{1b}[31mhi\u{202e}"), None, 32), "Bash echo [31mhi");
        assert_eq!(step("Bash", None, Some(&"x".repeat(50)), None, 10).chars().count(), 15);
    }

    #[test]
    fn the_line_reads_like_the_mockup() {
        let mut t = Ticker::default();
        for s in ["Read session.ts", "Edit session.ts", "Bash npm test", "Read refresh.ts"] {
            t.push(s.into(), 6);
        }
        assert_eq!(t.line(200).unwrap(), "Read session.ts → Edit session.ts → Bash npm test → Read refresh.ts →");
    }

    #[test]
    fn old_steps_fall_off_by_count_and_width() {
        let mut t = Ticker::default();
        for i in 0..10 {
            t.push(format!("Read f{i}.rs"), 3);
        }
        assert_eq!(t.line(200).unwrap(), "Read f7.rs → Read f8.rs → Read f9.rs →");
        assert_eq!(t.line(30).unwrap(), "Read f8.rs → Read f9.rs →");
        assert_eq!(t.line(5).unwrap().chars().count(), 5, "one long step is cut");
        t.clear();
        assert_eq!(t.line(200), None);
    }
}
