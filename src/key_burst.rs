//! Turn a flood of key events into one synthetic [`Event::Paste`].
//!
//! crossterm 0.28 reads Windows console input records, so it never produces
//! `Event::Paste`: Windows Terminal hands a paste over as one key press per
//! character, `Enter` per line break and `Tab` per tab. Routed one by one, each
//! `Enter` auto-indents and the pasted indent stacks on top of it.
//!
//! The approach follows Neovim, not its code. Neovim treats a paste as one
//! stream of raw text (`tinput_flush` / `handle_bracketed_paste` in
//! `src/nvim/tui/input.c`, phases 1/2/3 in `nvim_paste`), inserted without
//! mappings or auto-indent and joined into one undo step (`vim.paste` runs
//! `undojoin`), with CR and CRLF taken as line breaks. On Windows Neovim does not
//! guess: it turns on `ENABLE_VIRTUAL_TERMINAL_INPUT` (`src/nvim/event/stream.c`)
//! so the terminal's own `ESC[200~` markers arrive. crossterm cannot parse VT
//! input on Windows, so here the batch of keys that arrived together stands in
//! for those markers, and a short quiet window stands in for the end marker.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A paste ends after this long without another plain key press. Split
/// batches that arrive closer together than this join the same paste.
pub const PASTE_WINDOW: Duration = Duration::from_millis(30);
/// Plain presses in one batch needed before the batch can be a paste.
pub const PASTE_MIN_KEYS: usize = 2;
/// A batch this long is a paste even without a line break or tab.
/// Above any IME commit (a Thai syllable is 1-4 characters).
pub const PASTE_LONG_KEYS: usize = 32;
/// Most events drained in one go after a key event.
pub const MAX_DRAIN: usize = 16_384;

/// The text a plain press types, or `None` when the key is not plain text.
/// Shift is allowed; `Ctrl + Alt` together is `AltGr` on Windows and types too.
fn plain_text(key: &KeyEvent) -> Option<char> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl != alt {
        return None;
    }
    match key.code {
        KeyCode::Enter if key.modifiers.is_empty() => Some('\n'),
        KeyCode::Tab if key.modifiers.is_empty() => Some('\t'),
        KeyCode::Char('\r' | '\n') => Some('\n'),
        KeyCode::Char(c) if c == '\t' || !c.is_control() => Some(c),
        _ => None,
    }
}

fn is_release(event: &Event) -> bool {
    matches!(event, Event::Key(key) if key.kind == KeyEventKind::Release)
}

/// Drop `ESC [ 2 0 0 ~` / `ESC [ 2 0 1 ~` that leaked through as keys.
fn strip_marker_keys(events: Vec<Event>) -> Vec<Event> {
    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if let Event::Key(key) = &events[i]
            && key.code == KeyCode::Esc
        {
            let rest: String = events[i + 1..]
                .iter()
                .take(5)
                .map(|event| match event {
                    Event::Key(KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    }) => *c,
                    _ => '\0',
                })
                .collect();
            if rest == "[200~" || rest == "[201~" {
                i += 6;
                continue;
            }
        }
        out.push(events[i].clone());
        i += 1;
    }
    out
}

/// Remove marker text the terminal let through without its `ESC`.
fn strip_marker_text(text: &mut String) {
    for marker in ["\x1b[200~", "\x1b[201~"] {
        while let Some(at) = text.find(marker) {
            text.replace_range(at..at + marker.len(), "");
        }
    }
    if let Some(rest) = text.strip_prefix("[200~") {
        *text = rest.to_string();
    }
    if let Some(rest) = text.strip_suffix("[201~") {
        *text = rest.to_string();
    }
}

/// Collects drained input batches and decides what is typing and what is a paste.
#[derive(Debug, Default)]
pub struct KeyBurst {
    pending: Option<(String, Instant)>,
}

impl KeyBurst {
    /// When the pending paste is complete if nothing else arrives.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, last)| *last + PASTE_WINDOW)
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Emit the pending paste now.
    pub fn flush(&mut self) -> Option<Event> {
        let (mut text, _) = self.pending.take()?;
        strip_marker_text(&mut text);
        (!text.is_empty()).then_some(Event::Paste(text))
    }

    /// Emit the pending paste when its window has passed.
    pub fn flush_due(&mut self, now: Instant) -> Option<Event> {
        match self.deadline() {
            Some(due) if now >= due => self.flush(),
            _ => None,
        }
    }

    /// Feed one batch of events that were already queued together, in arrival order.
    /// Returns the events to route now, in order. Key releases are dropped (every
    /// router ignores them; Windows reports one per press). A paste stays pending
    /// until [`flush_due`](Self::flush_due), a non-text event, or [`flush`](Self::flush).
    pub fn push(&mut self, batch: Vec<Event>, now: Instant) -> Vec<Event> {
        let mut out = Vec::new();
        if let Some(event) = self.flush_due(now) {
            out.push(event);
        }
        let batch: Vec<Event> = batch
            .into_iter()
            .filter(|event| !is_release(event))
            .collect();
        let batch = strip_marker_keys(batch);
        let mut run: Vec<(Event, char)> = Vec::new();
        for event in batch {
            let text = match &event {
                Event::Key(key) => plain_text(key),
                _ => None,
            };
            match text {
                Some(ch) => run.push((event, ch)),
                None => {
                    self.take_run(std::mem::take(&mut run), now, &mut out);
                    if let Some(paste) = self.flush() {
                        out.push(paste);
                    }
                    out.push(event);
                }
            }
        }
        self.take_run(run, now, &mut out);
        out
    }

    fn take_run(&mut self, run: Vec<(Event, char)>, now: Instant, out: &mut Vec<Event>) {
        if run.is_empty() {
            return;
        }
        let continues = self.pending.is_some();
        if continues || Self::looks_pasted(&run) {
            let (text, last) = self.pending.get_or_insert_with(|| (String::new(), now));
            text.extend(run.iter().map(|(_, ch)| *ch));
            *last = now;
        } else {
            out.extend(run.into_iter().map(|(event, _)| event));
        }
    }

    /// Two or more presses with a line break or tab, or a long run. A run of one
    /// repeated key (a held key catching up) is still typing.
    fn looks_pasted(run: &[(Event, char)]) -> bool {
        if run.len() < PASTE_MIN_KEYS {
            return false;
        }
        if run.iter().all(|(_, ch)| *ch == run[0].1) {
            return false;
        }
        run.len() >= PASTE_LONG_KEYS || run.iter().any(|(_, ch)| matches!(ch, '\n' | '\t'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{AgentAction, CommandBarAction, Focus, SemanticAction};
    use crate::agent::PasteBurst;
    use crate::app::App;
    use crate::buffer::Buffer;

    /// The script from the bug report, rebuilt: 4-space indents, Thai comments.
    const SCRIPT: &str = "import bpy\n\
\n\
# เลือกวัตถุทั้งหมดในฉาก\n\
objects = bpy.context.scene.objects\n\
\n\
if len(objects) == 0:\n\
    print(\"ไม่มีวัตถุในฉาก\")\n\
else:\n\
    # นับจำนวน mesh\n\
    count = 0\n\
    for obj in objects:\n\
        if obj.type != 'MESH':\n\
            continue\n\
        count += 1\n\
        # ตั้งชื่อใหม่ตามลำดับ\n\
        obj.name = f\"Mesh_{count:03d}\"\n\
        for mod in obj.modifiers:\n\
            if mod.type == 'SUBSURF':\n\
                mod.levels = 2\n\
            else:\n\
                pass\n\
\n\
    print(f\"เปลี่ยนชื่อ {count} ชิ้น\")\n\
\n\
# ซ่อนวัตถุที่ไม่ใช่ mesh\n\
for obj in objects:\n\
    if obj.type == 'MESH':\n\
        continue\n\
    obj.hide_viewport = True\n\
\n\
bpy.context.view_layer.update()\n\
print(\"เสร็จแล้ว\")\n";

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn release(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ))
    }

    /// What Windows Terminal + crossterm deliver for a paste: a press and a release
    /// per character, `Enter` per line break (the CR of CRLF), `Tab` per tab.
    fn windows_keys(text: &str) -> Vec<Event> {
        let mut out = Vec::new();
        for ch in text.chars().filter(|ch| *ch != '\n') {
            let code = match ch {
                '\r' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                c => KeyCode::Char(c),
            };
            out.push(press(code));
            out.push(release(code));
        }
        out
    }

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    /// The same routing as `route_event` in `main.rs`.
    fn route(app: &mut App, events: Vec<Event>, agent: &mut PasteBurst) -> Vec<SemanticAction> {
        let mut done = Vec::new();
        for event in events {
            let action = crate::input::route_flags(
                &event,
                app.focus(),
                false,
                false,
                app.keys(),
                false,
                false,
                agent,
            );
            if let Some(action) = action {
                done.push(action.clone());
                app.handle(action);
            }
        }
        done
    }

    fn feed(app: &mut App, batches: Vec<Vec<Event>>) -> Vec<SemanticAction> {
        let mut burst = KeyBurst::default();
        let mut agent = PasteBurst::default();
        let start = Instant::now();
        let mut done = Vec::new();
        for (i, batch) in batches.into_iter().enumerate() {
            let at = start + Duration::from_millis(i as u64);
            let events = burst.push(batch, at);
            done.extend(route(app, events, &mut agent));
        }
        let events: Vec<Event> = burst.flush().into_iter().collect();
        done.extend(route(app, events, &mut agent));
        done
    }

    fn editor() -> App {
        App::new(Buffer::new(), Some(std::path::PathBuf::from("blender.py")))
    }

    fn undo_steps(app: &mut App) -> usize {
        let mut steps = 0;
        while app.buffer_mut().undo() {
            steps += 1;
        }
        steps
    }

    #[test]
    fn windows_paste_of_the_reported_script_is_the_script() {
        let mut app = editor();
        let done = feed(&mut app, vec![windows_keys(&crlf(SCRIPT))]);
        assert_eq!(done.len(), 1, "one paste, not one action per key");
        assert!(matches!(done[0], SemanticAction::InsertText(_)));
        assert_eq!(app.buffer().rope().to_string(), SCRIPT);
        assert_eq!(undo_steps(&mut app), 1);
        assert_eq!(app.buffer().rope().to_string(), "");
    }

    #[test]
    fn a_paste_split_over_several_batches_is_still_one_paste() {
        let keys = windows_keys(&crlf(SCRIPT));
        let batches: Vec<Vec<Event>> = keys.chunks(37).map(<[Event]>::to_vec).collect();
        assert!(batches.len() > 10);
        let mut app = editor();
        let done = feed(&mut app, batches);
        assert_eq!(done.len(), 1);
        assert_eq!(app.buffer().rope().to_string(), SCRIPT);
        assert_eq!(undo_steps(&mut app), 1);
    }

    #[test]
    fn a_late_batch_after_the_window_starts_its_own_paste() {
        let mut burst = KeyBurst::default();
        let start = Instant::now();
        assert!(burst.push(windows_keys("ab\r"), start).is_empty());
        assert_eq!(burst.deadline(), Some(start + PASTE_WINDOW));
        let late = start + PASTE_WINDOW + Duration::from_millis(1);
        let out = burst.push(windows_keys("cd\r"), late);
        assert_eq!(out, vec![Event::Paste("ab\n".into())]);
        assert_eq!(burst.flush(), Some(Event::Paste("cd\n".into())));
        assert_eq!(burst.flush_due(late), None);
    }

    #[test]
    fn the_pending_paste_flushes_only_when_due() {
        let mut burst = KeyBurst::default();
        let start = Instant::now();
        burst.push(windows_keys("x\ty"), start);
        assert!(burst.is_pending());
        assert_eq!(burst.flush_due(start + Duration::from_millis(5)), None);
        assert_eq!(
            burst.flush_due(start + PASTE_WINDOW),
            Some(Event::Paste("x\ty".into()))
        );
        assert!(!burst.is_pending());
    }

    #[test]
    fn single_keys_and_short_bursts_stay_typing() {
        let mut burst = KeyBurst::default();
        let now = Instant::now();
        // One key per batch, as when someone types.
        for code in [KeyCode::Char('a'), KeyCode::Enter, KeyCode::Tab] {
            assert_eq!(
                burst.push(vec![press(code), release(code)], now),
                vec![press(code)]
            );
        }
        // A Thai IME commit: several characters at once, no line break.
        let thai: Vec<Event> = "กำลัง".chars().map(|c| press(KeyCode::Char(c))).collect();
        assert_eq!(burst.push(thai.clone(), now), thai);
        // A held key catching up is not a paste.
        let held = vec![press(KeyCode::Enter); 3];
        assert_eq!(burst.push(held.clone(), now), held);
        assert!(!burst.is_pending());
    }

    #[test]
    fn a_long_run_without_line_breaks_is_a_paste() {
        let mut burst = KeyBurst::default();
        let text: String = "abcdefghij".repeat(4);
        let keys: Vec<Event> = text.chars().map(|c| press(KeyCode::Char(c))).collect();
        let short = keys[..PASTE_LONG_KEYS - 1].to_vec();
        assert_eq!(burst.push(short.clone(), Instant::now()), short);
        assert!(burst.push(keys, Instant::now()).is_empty());
        assert_eq!(burst.flush(), Some(Event::Paste(text)));
    }

    #[test]
    fn typing_in_the_editor_still_auto_indents() {
        let mut app = editor();
        let typed = "if x:\r    y\r";
        let batches: Vec<Vec<Event>> = windows_keys(typed)
            .chunks(2)
            .map(<[Event]>::to_vec)
            .collect();
        // Spread the keys out so each one is typed alone.
        let mut burst = KeyBurst::default();
        let mut agent = PasteBurst::default();
        let start = Instant::now();
        for (i, batch) in batches.into_iter().enumerate() {
            let at = start + Duration::from_millis(100 * i as u64);
            let events = burst.push(batch, at);
            route(&mut app, events, &mut agent);
        }
        assert!(!burst.is_pending());
        assert_eq!(app.buffer().rope().to_string(), "if x:\n    y\n    ");
    }

    #[test]
    fn press_and_release_give_each_character_once() {
        let mut burst = KeyBurst::default();
        let out = burst.push(windows_keys("ab\r"), Instant::now());
        assert!(out.is_empty());
        assert_eq!(burst.flush(), Some(Event::Paste("ab\n".into())));
        // A release left over from a key typed earlier changes nothing.
        let out = burst.push(vec![release(KeyCode::Char('a'))], Instant::now());
        assert!(out.is_empty());
        assert!(!burst.is_pending());
        // Repeat counts as a press.
        let repeat = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('z'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        assert_eq!(
            burst.push(vec![repeat.clone()], Instant::now()),
            vec![repeat]
        );
    }

    #[test]
    fn other_events_keep_their_place() {
        let mut burst = KeyBurst::default();
        let mut batch = windows_keys("a\rb");
        batch.push(Event::Resize(80, 24));
        batch.extend(windows_keys("c"));
        batch.push(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        let out = burst.push(batch, Instant::now());
        assert_eq!(
            out,
            vec![
                Event::Paste("a\nb".into()),
                Event::Resize(80, 24),
                press(KeyCode::Char('c')),
                Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            ]
        );
        // A real bracketed paste passes straight through.
        let real = Event::Paste("x\r\ny".into());
        assert_eq!(burst.push(vec![real.clone()], Instant::now()), vec![real]);
    }

    #[test]
    fn leaked_bracketed_paste_markers_are_dropped() {
        let mut burst = KeyBurst::default();
        let mut batch = vec![press(KeyCode::Esc), release(KeyCode::Esc)];
        batch.extend(windows_keys("[200~if a:\r    b"));
        batch.extend([press(KeyCode::Esc)]);
        batch.extend(windows_keys("[201~"));
        assert!(burst.push(batch, Instant::now()).is_empty());
        assert_eq!(burst.flush(), Some(Event::Paste("if a:\n    b".into())));
        // Without the ESC, only at the ends.
        burst.push(windows_keys("[200~x\ry[201~"), Instant::now());
        assert_eq!(burst.flush(), Some(Event::Paste("x\ny".into())));
    }

    #[test]
    fn altgr_characters_are_text_but_ctrl_chords_are_not() {
        let altgr = Event::Key(KeyEvent::new(
            KeyCode::Char('{'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        let mut burst = KeyBurst::default();
        let mut batch = vec![altgr];
        batch.extend(windows_keys("\r}"));
        assert!(burst.push(batch, Instant::now()).is_empty());
        assert_eq!(burst.flush(), Some(Event::Paste("{\n}".into())));
    }

    #[test]
    fn the_agent_prompt_gets_the_whole_paste_without_sending() {
        let mut app = editor();
        app.agent.open = true;
        app.agent.focused = true;
        assert_eq!(app.focus(), Focus::Agent);
        let done = feed(&mut app, vec![windows_keys(&crlf(SCRIPT))]);
        assert!(!done.contains(&SemanticAction::Agent(AgentAction::Submit)));
        assert_eq!(app.agent.prompt.to_string(), SCRIPT);
    }

    #[test]
    fn the_command_bar_does_not_run_a_pasted_line() {
        let mut app = editor();
        app.handle(SemanticAction::OpenCommandBar);
        assert_eq!(app.focus(), Focus::CommandBar);
        let done = feed(&mut app, vec![windows_keys("goto 3\r\nquit\r\n")]);
        assert!(!done.contains(&SemanticAction::CommandBar(CommandBarAction::Submit)));
        assert_eq!(app.focus(), Focus::CommandBar);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "goto 3quit");
    }
}
