//! Live ticker: each running agent row's meta line becomes a slow ticker of what it is doing now.
// wit-bindgen's flattened `notify` import has 14 params.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({ path: "wit/haunt-plugin.wit", world: "plugin" });

mod ticker;

use std::collections::BTreeMap;
use std::sync::Mutex;

use haunt::plugin::log::log;
use haunt::plugin::rows::{self, Decoration, Tone};
use haunt::plugin::types::LogLevel;
use serde::Deserialize;
use ticker::{Ticker, step};

/// The host cuts a suffix at 64 characters, or 160 when it scrolls.
const WIDTH: usize = 64;
const SCROLL_WIDTH: usize = 160;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub steps: i64,
    /// Let the line scroll slowly when it doesn't fit.
    pub scroll: bool,
    /// Keep the last steps on the row after the turn ends.
    pub keep_after_stop: bool,
    pub subagents: bool,
    pub unverified: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { steps: 6, scroll: true, keep_after_stop: false, subagents: false, unverified: false }
    }
}

#[derive(Default)]
struct Live {
    settings: Settings,
    tabs: BTreeMap<String, Ticker>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Live) -> R) -> R {
    f(LIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Live::default))
}

pub fn decoration(t: &Ticker, s: &Settings) -> Option<Decoration> {
    let line = t.line(if s.scroll { SCROLL_WIDTH } else { WIDTH })?;
    Some(Decoration { label: None, label_tone: Tone::Dim, suffix: Some(line), suffix_tone: Tone::Dim, scroll: s.scroll, bar: None, spark: None })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Step(String),
    Clear,
}

pub fn change(e: &AgentEvent, s: &Settings) -> Option<Change> {
    match e.kind.as_str() {
        "tool_start" if e.agent_id.is_none() || s.subagents => {
            Some(Change::Step(step(e.tool.as_deref().unwrap_or("tool"), e.file_path.as_deref(), e.command.as_deref(), None, 32)))
        }
        "prompt_submit" => Some(Change::Clear),
        "stop" | "stop_failure" if !s.keep_after_stop => Some(Change::Clear),
        _ => None,
    }
}

struct Plugin;

impl Guest for Plugin {
    fn init(json: String) {
        with(|l| l.settings = serde_json::from_str(&json).unwrap_or_default());
    }

    fn on_settings_changed(json: String) {
        with(|l| {
            l.settings = serde_json::from_str(&json).unwrap_or_default();
            for (s, t) in &l.tabs {
                let _ = rows::decorate(s, decoration(t, &l.settings).as_ref());
            }
        });
    }

    fn on_event(event: AgentEvent) {
        let Some(surface) = event.surface.clone() else { return };
        with(|l| {
            if !event.verified && !l.settings.unverified {
                return;
            }
            let Some(c) = change(&event, &l.settings) else { return };
            let keep = l.settings.steps.clamp(1, 12) as usize;
            let t = l.tabs.entry(surface.clone()).or_default();
            match c {
                Change::Step(s) => t.push(s, keep),
                Change::Clear => t.clear(),
            }
            if let Err(e) = rows::decorate(&surface, decoration(t, &l.settings).as_ref()) {
                log(LogLevel::Debug, &format!("live-ticker: {surface}: {e}"));
                l.tabs.remove(&surface);
            }
        });
    }

    fn palette_items(_mode: String, _query: String) -> Vec<PaletteItem> {
        Vec::new()
    }

    fn on_action(_action_id: String, _payload: String) {}

    fn row_menu(_surface: String) -> Vec<MenuItem> {
        Vec::new()
    }

    fn on_row_moved(_surface: String, _group: String, _before: Option<String>) {}

    fn on_theme_changed() {}

    fn on_tick(_now_ms: u64) {}

    fn on_exec(_job: u64, _outcome: Result<haunt::plugin::exec::Output, String>) {}

    fn on_away_changed(_away: bool, _away_ms: u64) {}

    fn on_hover_hold(_held: bool, _surface: String) {}

    fn on_fs_tree(_job: u64, _outcome: Result<haunt::plugin::fs_list::TreeResult, String>) {}
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, tool: Option<&str>, file: Option<&str>, command: Option<&str>) -> AgentEvent {
        AgentEvent {
            kind: kind.into(),
            session_id: None,
            agent_id: None,
            cwd: None,
            summary: None,
            verified: true,
            surface: Some("s".into()),
            tool: tool.map(Into::into),
            command: command.map(Into::into),
            file_path: file.map(Into::into),
            task_id: None,
            request_key: None,
        }
    }

    #[test]
    fn the_meta_line_reads_like_the_mockup() {
        let s = Settings::default();
        let mut t = Ticker::default();
        for e in [
            ev("tool_start", Some("Read"), Some("/w/web/session.ts"), None),
            ev("tool_start", Some("Edit"), Some("/w/web/session.ts"), None),
            ev("tool_start", Some("Bash"), None, Some("npm test")),
            ev("tool_start", Some("Read"), Some("/w/web/refresh.ts"), None),
        ] {
            let Some(Change::Step(st)) = change(&e, &s) else { panic!() };
            t.push(st, 6);
        }
        let d = decoration(&t, &s).unwrap();
        assert_eq!(d.suffix.as_deref(), Some("Read session.ts → Edit session.ts → Bash npm test → Read refresh.ts →"));
        assert!(d.scroll && d.label.is_none());
        let still = decoration(&t, &Settings { scroll: false, ..s }).unwrap();
        assert_eq!(still.suffix.as_deref(), Some("Edit session.ts → Bash npm test → Read refresh.ts →"), "the oldest step goes to fit 64");
    }

    #[test]
    fn a_new_turn_or_its_end_clears_it() {
        let s = Settings::default();
        assert_eq!(change(&ev("prompt_submit", None, None, None), &s), Some(Change::Clear));
        assert_eq!(change(&ev("stop", None, None, None), &s), Some(Change::Clear));
        assert_eq!(change(&ev("stop", None, None, None), &Settings { keep_after_stop: true, ..s.clone() }), None);
        assert_eq!(change(&ev("tool_ran", Some("Read"), None, None), &s), None, "each call once, as it starts");
        assert!(decoration(&Ticker::default(), &s).is_none());
    }

    #[test]
    fn subagent_calls_only_when_asked() {
        let mut e = ev("tool_start", Some("Grep"), None, None);
        e.agent_id = Some("sub".into());
        assert_eq!(change(&e, &Settings::default()), None);
        assert_eq!(change(&e, &Settings { subagents: true, ..Settings::default() }), Some(Change::Step("Grep".into())));
    }

    #[test]
    fn manifest_asks_for_what_it_uses_and_defaults_match() {
        let m = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/haunt-plugin.toml")).unwrap();
        let t: toml::Table = toml::from_str(&m).unwrap();
        let caps = t["capabilities"].as_table().unwrap();
        assert_eq!(caps.keys().collect::<Vec<_>>(), ["decorate", "event_details", "events"]);
        let defaults: serde_json::Map<String, serde_json::Value> =
            t["settings"].as_table().unwrap().iter().map(|(k, v)| (k.clone(), serde_json::to_value(&v["default"]).unwrap())).collect();
        assert_eq!(serde_json::from_value::<Settings>(serde_json::Value::Object(defaults)).unwrap(), Settings::default());
    }
}
