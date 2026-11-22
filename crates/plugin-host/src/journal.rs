use std::collections::VecDeque;
use std::sync::Mutex;

use craft_plugin::LogLevel;
use serde::Serialize;

const KEPT: usize = 512;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    pub seq: u64,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Default)]
pub(crate) struct Journal {
    lines: Mutex<(u64, VecDeque<LogLine>)>,
}

impl Journal {
    pub fn push(&self, level: LogLevel, message: String) {
        let Ok(mut held) = self.lines.lock() else { return };
        let (next, lines) = &mut *held;
        if lines.back().is_some_and(|last| last.message == message) {
            return;
        }
        *next += 1;
        if lines.len() == KEPT {
            lines.pop_front();
        }
        lines.push_back(LogLine { seq: *next, level, message });
    }

    pub fn after(&self, seq: u64) -> Vec<LogLine> {
        self.lines.lock().map(|held| held.1.iter().filter(|line| line.seq > seq).cloned().collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_after_a_sequence_number_are_returned_and_only_the_newest_are_kept() {
        let journal = Journal::default();
        for n in 0..KEPT + 2 {
            journal.push(LogLevel::Info, format!("line {n}"));
        }
        let after = journal.after(KEPT as u64);
        assert_eq!(after.iter().map(|line| line.message.as_str()).collect::<Vec<_>>(), [format!("line {}", KEPT), format!("line {}", KEPT + 1)]);
        assert_eq!(journal.after(0).len(), KEPT);
        assert_eq!(journal.after(0)[0].seq, 3);
    }
}
