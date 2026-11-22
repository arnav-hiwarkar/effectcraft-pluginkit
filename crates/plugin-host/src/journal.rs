use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use craft_plugin::LogLevel;
use serde::Serialize;

const KEPT: usize = 512;
const KEPT_BYTES: usize = 64 << 10;
const LONGEST: usize = 4 << 10;

static LOADS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    pub seq: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogPage {
    pub load: u64,
    pub missed: u64,
    pub lines: Vec<LogLine>,
}

#[derive(Default)]
struct Held {
    next: u64,
    bytes: usize,
    lines: VecDeque<LogLine>,
}

pub(crate) struct Journal {
    load: u64,
    held: Mutex<Held>,
}

impl Default for Journal {
    fn default() -> Self {
        Journal { load: LOADS.fetch_add(1, Ordering::Relaxed) + 1, held: Mutex::default() }
    }
}

fn one_row(message: &str) -> String {
    let mut row = String::with_capacity(message.len().min(LONGEST));
    for c in message.chars() {
        if row.len() >= LONGEST {
            row.push('…');
            break;
        }
        match c {
            '\n' => row.push_str("\\n"),
            '\r' => row.push_str("\\r"),
            '\t' => row.push_str("\\t"),
            c if c.is_control() => row.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => row.push(c),
        }
    }
    row
}

impl Journal {
    pub fn push(&self, level: LogLevel, message: String) {
        let Ok(mut held) = self.held.lock() else { return };
        let message = one_row(&message);
        if held.lines.back().is_some_and(|last| last.level == level && last.message == message) {
            return;
        }
        held.next += 1;
        held.bytes += message.len();
        let seq = held.next;
        held.lines.push_back(LogLine { seq, level, message });
        while held.lines.len() > KEPT || held.bytes > KEPT_BYTES {
            let Some(dropped) = held.lines.pop_front() else { break };
            held.bytes -= dropped.message.len();
        }
    }

    pub fn latest(&self) -> u64 {
        self.held.lock().map_or(0, |held| held.next)
    }

    pub fn page(&self, after: u64, load: Option<u64>) -> LogPage {
        let after = if load.is_none_or(|load| load == self.load) { after } else { 0 };
        let Ok(held) = self.held.lock() else { return LogPage { load: self.load, missed: 0, lines: Vec::new() } };
        let first = held.lines.front().map_or(held.next + 1, |line| line.seq);
        let lines = held.lines.iter().filter(|line| line.seq > after).cloned().collect();
        LogPage { load: self.load, missed: first.saturating_sub(after + 1), lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(page: &LogPage) -> Vec<&str> {
        page.lines.iter().map(|line| line.message.as_str()).collect()
    }

    #[test]
    fn lines_after_a_sequence_number_are_returned_and_only_the_newest_are_kept() {
        let journal = Journal::default();
        for n in 0..KEPT + 2 {
            journal.push(LogLevel::Info, format!("line {n}"));
        }
        let after = journal.page(KEPT as u64, None);
        assert_eq!(messages(&after), [format!("line {}", KEPT), format!("line {}", KEPT + 1)]);
        assert_eq!(journal.page(0, None).lines.len(), KEPT);
        assert_eq!(journal.page(0, None).lines[0].seq, 3);
    }

    #[test]
    fn the_log_keeps_at_most_64_kib_and_a_reader_that_fell_behind_is_told_how_many_lines_it_missed() {
        let journal = Journal::default();
        for n in 0..100 {
            journal.push(LogLevel::Info, format!("{n:04}{}", "x".repeat(3996)));
        }
        let page = journal.page(0, None);
        assert_eq!(page.lines.len(), 16);
        assert_eq!(page.lines[0].seq, 85);
        assert_eq!(page.missed, 84);
        assert_eq!(journal.page(90, None).missed, 0);
        journal.push(LogLevel::Info, "y".repeat(100_000));
        assert_eq!(journal.page(100, None).lines[0].message.len(), LONGEST + '…'.len_utf8());
    }

    #[test]
    fn newlines_and_control_characters_are_escaped_so_one_message_is_one_row() {
        let journal = Journal::default();
        journal.push(LogLevel::Info, "done\n[error] forged\r\t\u{1b}[31m".into());
        assert_eq!(messages(&journal.page(0, None)), ["done\\n[error] forged\\r\\t\\u{1b}[31m"]);
    }

    #[test]
    fn the_same_text_at_another_level_is_a_new_entry() {
        let journal = Journal::default();
        journal.push(LogLevel::Info, "x".into());
        journal.push(LogLevel::Info, "x".into());
        journal.push(LogLevel::Error, "x".into());
        let levels: Vec<LogLevel> = journal.page(0, None).lines.into_iter().map(|line| line.level).collect();
        assert_eq!(levels, [LogLevel::Info, LogLevel::Error]);
    }

    #[test]
    fn a_cursor_from_an_earlier_load_reads_the_new_load_from_its_start() {
        let earlier = Journal::default();
        let reloaded = Journal::default();
        reloaded.push(LogLevel::Info, "started again".into());
        let page = reloaded.page(40, Some(earlier.load));
        assert_eq!((page.load, messages(&page)), (reloaded.load, vec!["started again"]));
        assert_eq!(reloaded.page(40, Some(reloaded.load)).lines, []);
    }
}
