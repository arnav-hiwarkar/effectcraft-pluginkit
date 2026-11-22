use std::time::Duration;

use craft_plugin::{AppEvent, MIN_TIMER_MS, PanelReply, Pointer, PointerPhase, Topic, UiEvent, UiEventKind, UiInput, Widget};

const LEGACY_TIMER: &str = "";

#[derive(Debug, Clone, PartialEq)]
struct Armed {
    name: String,
    interval: Duration,
    repeat: bool,
    due: Duration,
}

#[derive(Debug, Default)]
pub struct Retained {
    widgets: Vec<Widget>,
    error: Option<String>,
    events: Vec<UiEvent>,
    keys: String,
    app: Vec<AppEvent>,
    topics: Vec<Topic>,
    timers: Vec<Armed>,
    size: [f32; 2],
    drawn: bool,
    stale: bool,
    hidden: bool,
    in_flight: bool,
    legacy: bool,
    calls: u64,
}

fn merged(pending: &UiEventKind, next: &UiEventKind) -> Option<UiEventKind> {
    match (pending, next) {
        (UiEventKind::Pointer(before), UiEventKind::Pointer(after))
            if before.phase == after.phase && matches!(after.phase, PointerPhase::Hover | PointerPhase::Drag) && before.modifiers == after.modifiers =>
        {
            let scroll = [before.scroll[0] + after.scroll[0], before.scroll[1] + after.scroll[1]];
            Some(UiEventKind::Pointer(Pointer { scroll, ..*after }))
        }
        (UiEventKind::Number(_), UiEventKind::Number(_)) | (UiEventKind::Text(_), UiEventKind::Text(_)) | (UiEventKind::Color(_), UiEventKind::Color(_)) => {
            Some(next.clone())
        }
        _ => None,
    }
}

fn one_call(events: &mut Vec<UiEvent>) -> Vec<UiEvent> {
    let mut acted = Vec::new();
    let collapses = |event: &UiEvent| matches!(event.kind, UiEventKind::Click | UiEventKind::Bool(_) | UiEventKind::Drop(_));
    let end = events
        .iter()
        .position(|event| {
            let again = collapses(event) && acted.contains(&event.id);
            if collapses(event) {
                acted.push(event.id);
            }
            again
        })
        .unwrap_or(events.len());
    events.drain(..end).collect()
}

impl Retained {
    pub fn new() -> Self {
        Retained::default()
    }

    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn calls(&self) -> u64 {
        self.calls
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn pending(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn subscribed(&self, topic: Topic) -> bool {
        self.topics.contains(&topic)
    }

    pub fn push(&mut self, events: Vec<UiEvent>, keys: &str) {
        let in_flight = self.in_flight;
        for event in events {
            let last = self.events.iter_mut().rev().find(|pending| pending.id == event.id);
            match last.filter(|_| in_flight).and_then(|last| merged(&last.kind, &event.kind).map(|kind| (last, kind))) {
                Some((last, kind)) => last.kind = kind,
                None => self.events.push(event),
            }
        }
        self.keys.push_str(keys);
    }

    pub fn notify(&mut self, event: &AppEvent) {
        if !self.subscribed(event.topic()) {
            return;
        }
        match (self.app.iter_mut().find(|pending| pending.topic() == event.topic()), event) {
            (Some(AppEvent::Edit { layers: pending }), AppEvent::Edit { layers }) => {
                pending.extend(layers);
                pending.sort_unstable();
                pending.dedup();
            }
            (Some(pending), _) => *pending = event.clone(),
            (None, _) => self.app.push(event.clone()),
        }
    }

    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    pub fn hide(&mut self) {
        self.hidden = true;
        self.events.clear();
        self.keys.clear();
    }

    pub fn next_due(&self) -> Option<Duration> {
        if self.hidden || self.in_flight {
            return None;
        }
        self.timers.iter().map(|timer| timer.due).min()
    }

    pub fn start(&mut self, size: [f32; 2], now: Duration) -> Option<UiInput> {
        if self.in_flight {
            return None;
        }
        if std::mem::take(&mut self.hidden) {
            self.timers.iter_mut().for_each(|timer| timer.due = now + timer.interval);
        }
        let fired: Vec<String> = self.timers.iter().filter(|timer| timer.due <= now).map(|timer| timer.name.clone()).collect();
        let triggered = !self.drawn || self.stale || size != self.size || !self.events.is_empty() || !self.keys.is_empty() || !self.app.is_empty();
        if !triggered && fired.is_empty() {
            return None;
        }
        self.timers.retain_mut(|timer| {
            if timer.due > now {
                return true;
            }
            while timer.due <= now {
                timer.due += timer.interval;
            }
            timer.repeat
        });
        let mut events = one_call(&mut self.events);
        if self.legacy {
            events.retain(|event| event.kind.known_to_every_sdk());
        }
        let timers = fired.into_iter().filter(|name| name != LEGACY_TIMER).collect();
        let input = UiInput { events, size, keys: std::mem::take(&mut self.keys), app: std::mem::take(&mut self.app), timers };
        (self.size, self.stale, self.in_flight) = (size, false, true);
        self.calls += 1;
        Some(input)
    }

    pub fn abandon(&mut self) {
        (self.in_flight, self.stale) = (false, true);
    }

    pub fn finish(&mut self, reply: Result<PanelReply, String>, now: Duration) {
        if self.hidden {
            return self.abandon();
        }
        self.in_flight = false;
        self.drawn = true;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.error = None;
        self.widgets = reply.widgets;
        self.topics = reply.topics;
        self.legacy = reply.legacy;
        let mut wanted = reply.timers;
        if reply.legacy {
            wanted.push(craft_plugin::Timer { name: LEGACY_TIMER.into(), ms: MIN_TIMER_MS, repeat: true });
        }
        let before = std::mem::take(&mut self.timers);
        self.timers = wanted
            .into_iter()
            .map(|timer| {
                let interval = Duration::from_millis(u64::from(timer.ms.max(MIN_TIMER_MS)));
                let kept = before.iter().find(|armed| armed.name == timer.name && armed.interval == interval && armed.repeat == timer.repeat);
                Armed { due: kept.map_or(now + interval, |armed| armed.due), name: timer.name, interval, repeat: timer.repeat }
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_plugin::{Key, Modifiers, TextStyle, Timer};

    const SIZE: [f32; 2] = [300.0, 200.0];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn label(text: &str) -> Widget {
        Widget::Label { text: text.into(), style: TextStyle::Body }
    }

    fn reply(timers: Vec<Timer>, topics: Vec<Topic>) -> Result<PanelReply, String> {
        Ok(PanelReply { widgets: vec![label("drawn")], topics, timers, legacy: false })
    }

    fn drawn(timers: Vec<Timer>, topics: Vec<Topic>) -> Retained {
        let mut panel = Retained::new();
        panel.start(SIZE, ms(0));
        panel.finish(reply(timers, topics), ms(0));
        panel
    }

    fn click(id: u64) -> UiEvent {
        UiEvent { id, kind: UiEventKind::Click }
    }

    fn pointer(id: u64, x: f32, phase: PointerPhase) -> UiEvent {
        UiEvent { id, kind: UiEventKind::Pointer(Pointer { scroll: [0.0, 1.0], ..Pointer::new([x, 0.0], phase) }) }
    }

    fn slow_call(panel: &mut Retained) {
        panel.invalidate();
        assert!(panel.start(SIZE, ms(16)).is_some());
    }

    fn events(panel: &mut Retained, now: u64) -> Option<Vec<UiEvent>> {
        let input = panel.start(SIZE, ms(now))?;
        panel.finish(reply(vec![], vec![]), ms(now));
        Some(input.events)
    }

    #[test]
    fn a_drawn_panel_is_not_called_again_until_something_happens() {
        let mut panel = drawn(vec![], vec![]);
        assert_eq!((0..100).filter_map(|frame| panel.start(SIZE, ms(16 * frame))).count(), 0);
        panel.push(vec![click(7)], "");
        assert_eq!(panel.start(SIZE, ms(1700)).map(|input| input.events), Some(vec![click(7)]));
        assert_eq!(panel.start(SIZE, ms(1716)), None);
    }

    #[test]
    fn two_clicks_on_one_button_during_a_slow_call_reach_two_calls_in_order() {
        let mut panel = drawn(vec![], vec![]);
        slow_call(&mut panel);
        panel.push(vec![click(7), UiEvent { id: 9, kind: UiEventKind::Number(2.0) }], "");
        panel.push(vec![click(7), click(8)], "");
        panel.finish(reply(vec![], vec![]), ms(40));
        assert!(panel.pending());
        assert_eq!(events(&mut panel, 48), Some(vec![click(7), UiEvent { id: 9, kind: UiEventKind::Number(2.0) }]));
        assert_eq!(events(&mut panel, 64), Some(vec![click(7), click(8)]));
        assert_eq!(events(&mut panel, 80), None);
    }

    #[test]
    fn pointer_motion_and_values_made_during_a_call_coalesce_and_presses_stay_in_order() {
        let mut panel = drawn(vec![], vec![]);
        slow_call(&mut panel);
        for x in 0..1000 {
            panel.push(vec![pointer(3, x as f32, PointerPhase::Drag), UiEvent { id: 4, kind: UiEventKind::Number(f64::from(x)) }], "");
        }
        panel.push(vec![pointer(3, 5.0, PointerPhase::Up), pointer(3, 6.0, PointerPhase::Hover), pointer(3, 7.0, PointerPhase::Hover)], "");
        panel.finish(reply(vec![], vec![]), ms(40));
        let drag = UiEvent { id: 3, kind: UiEventKind::Pointer(Pointer { scroll: [0.0, 1000.0], ..Pointer::new([999.0, 0.0], PointerPhase::Drag) }) };
        let hover = UiEvent { id: 3, kind: UiEventKind::Pointer(Pointer { scroll: [0.0, 2.0], ..Pointer::new([7.0, 0.0], PointerPhase::Hover) }) };
        assert_eq!(events(&mut panel, 48), Some(vec![drag, UiEvent { id: 4, kind: UiEventKind::Number(999.0) }, pointer(3, 5.0, PointerPhase::Up), hover]));
    }

    #[test]
    fn a_frame_of_events_made_while_idle_reaches_the_panel_whole() {
        let mut panel = drawn(vec![], vec![]);
        let moves: Vec<UiEvent> = [20.0, 30.0, 40.0].map(|x| pointer(3, x, PointerPhase::Drag)).to_vec();
        panel.push(moves.clone(), "");
        assert_eq!(events(&mut panel, 16), Some(moves));
    }

    #[test]
    fn the_events_of_a_failed_call_are_consumed_and_never_delivered_again() {
        let mut panel = drawn(vec![], vec![]);
        panel.push(vec![click(7)], "");
        panel.start(SIZE, ms(16));
        panel.finish(Err("trapped".into()), ms(16));
        assert_eq!(panel.error(), Some("trapped"));
        assert_eq!(panel.widgets(), &[label("drawn")]);
        assert_eq!(panel.start(SIZE, ms(32)), None);
        panel.push(vec![click(8)], "");
        assert_eq!(events(&mut panel, 48), Some(vec![click(8)]));
    }

    #[test]
    fn a_reply_that_arrives_after_the_panel_was_hidden_is_discarded_and_the_panel_is_called_again_when_shown() {
        let mut panel = drawn(vec![], vec![]);
        panel.push(vec![click(7)], "");
        panel.start(SIZE, ms(16));
        panel.hide();
        panel.finish(Ok(PanelReply { widgets: vec![label("late")], ..Default::default() }), ms(20));
        assert_eq!(panel.widgets(), &[label("drawn")]);
        assert_eq!(panel.start(SIZE, ms(32)).map(|input| input.events), Some(vec![]));
    }

    #[test]
    fn a_click_made_before_the_panel_is_hidden_is_dropped_and_timers_wait_while_it_is_hidden() {
        let mut panel = drawn(vec![Timer { name: "tick".into(), ms: 100, repeat: true }], vec![]);
        panel.push(vec![click(7)], "x");
        panel.hide();
        assert_eq!(panel.next_due(), None);
        assert_eq!(panel.start(SIZE, ms(5000)), None);
        assert_eq!(panel.next_due(), Some(ms(5100)));
        assert_eq!(panel.start(SIZE, ms(5100)).map(|input| (input.events, input.keys, input.timers)), Some((vec![], String::new(), vec!["tick".into()])));
    }

    #[test]
    fn app_events_reach_only_subscribed_panels_once_per_kind_with_the_latest_contents() {
        let mut panel = drawn(vec![], vec![Topic::Time]);
        panel.notify(&AppEvent::Comp { comp: Some(2) });
        assert_eq!(panel.start(SIZE, ms(16)), None);
        panel.notify(&AppEvent::Time { comp: 2, time: 1.0 });
        panel.notify(&AppEvent::Time { comp: 2, time: 1.5 });
        assert_eq!(panel.start(SIZE, ms(32)).map(|input| input.app), Some(vec![AppEvent::Time { comp: 2, time: 1.5 }]));
    }

    #[test]
    fn edits_made_while_a_panel_waits_union_their_layers() {
        let mut panel = drawn(vec![], vec![Topic::Edit]);
        panel.notify(&AppEvent::Edit { layers: vec![4, 1] });
        panel.notify(&AppEvent::Edit { layers: vec![2, 4] });
        assert_eq!(panel.start(SIZE, ms(16)).map(|input| input.app), Some(vec![AppEvent::Edit { layers: vec![1, 2, 4] }]));
    }

    #[test]
    fn a_repeating_timer_fires_ten_times_in_a_simulated_second_and_a_one_shot_once() {
        let timers = vec![Timer { name: "tick".into(), ms: 100, repeat: true }, Timer { name: "once".into(), ms: 250, repeat: false }];
        let mut panel = drawn(timers.clone(), vec![]);
        let mut fired: Vec<String> = Vec::new();
        for frame in 1..=60 {
            let now = Duration::from_secs_f64(f64::from(frame) / 60.0);
            if let Some(input) = panel.start(SIZE, now) {
                let once_done = fired.iter().any(|name| name == "once");
                fired.extend(input.timers.iter().cloned());
                let restated = timers.iter().filter(|timer| timer.repeat || !once_done).cloned().collect();
                panel.finish(reply(restated, vec![]), now);
            }
        }
        assert_eq!(fired.iter().filter(|name| *name == "tick").count(), 10);
        assert_eq!(fired.iter().filter(|name| *name == "once").count(), 1);
    }

    #[test]
    fn a_timer_below_the_floor_runs_at_sixteen_milliseconds() {
        let mut panel = drawn(vec![Timer { name: "fast".into(), ms: 1, repeat: true }], vec![]);
        assert_eq!(panel.next_due(), Some(ms(16)));
        assert_eq!(panel.start(SIZE, ms(15)), None);
        assert!(panel.start(SIZE, ms(16)).is_some());
    }

    #[test]
    fn a_panel_from_an_older_sdk_is_redrawn_every_frame_and_never_gets_event_kinds_it_cannot_read() {
        let mut panel = Retained::new();
        panel.start(SIZE, ms(0));
        panel.finish(Ok(PanelReply { widgets: vec![], legacy: true, ..Default::default() }), ms(0));
        let key = UiEvent { id: 3, kind: UiEventKind::Key(Key { name: "A".into(), modifiers: Modifiers::default() }) };
        panel.push(vec![key, click(3)], "");
        let input = panel.start(SIZE, ms(16)).unwrap_or_default();
        assert_eq!((input.events, input.timers), (vec![click(3)], vec![]));
        panel.finish(Ok(PanelReply { legacy: true, ..Default::default() }), ms(16));
        assert!(panel.start(SIZE, ms(32)).is_some());
    }
}
