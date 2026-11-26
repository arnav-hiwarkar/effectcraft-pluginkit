use effectcraft_plugin::viewer::Pen;

const TABLET_POINT: usize = 23;
const TABLET_PROXIMITY: usize = 24;
const MOUSE: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 25, 26, 27];
const SUBTYPE_TABLET_POINT: i16 = 1;
const SUBTYPE_TABLET_PROXIMITY: i16 = 2;
const DEVICE_CURSOR: usize = 2;
const DEVICE_ERASER: usize = 3;
const MOST_TILT: f64 = std::f64::consts::FRAC_PI_3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RawEvent {
    kind: usize,
    subtype: i16,
    pressure: f32,
    tilt: [f64; 2],
    device: usize,
    entering: bool,
}

impl RawEvent {
    fn mouse(&self) -> bool {
        MOUSE.contains(&self.kind)
    }

    fn proximity(&self) -> bool {
        self.kind == TABLET_PROXIMITY || (self.mouse() && self.subtype == SUBTYPE_TABLET_PROXIMITY)
    }

    fn point(&self) -> bool {
        self.kind == TABLET_POINT || (self.mouse() && self.subtype == SUBTYPE_TABLET_POINT)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Update {
    Keep,
    Set(Option<Pen>),
}

fn update(eraser: &mut bool, event: &RawEvent) -> Update {
    if event.proximity() {
        *eraser = event.entering && event.device == DEVICE_ERASER;
        let near = event.entering && event.device != DEVICE_CURSOR;
        return Update::Set(near.then_some(Pen { pressure: 0.0, tilt: [0.0; 2], eraser: *eraser }));
    }
    if event.point() {
        let tilt = event.tilt.map(|tilt| (tilt.clamp(-1.0, 1.0) * MOST_TILT) as f32);
        return Update::Set(Some(Pen { pressure: event.pressure, tilt, eraser: *eraser }));
    }
    if event.mouse() { Update::Set(None) } else { Update::Keep }
}

fn event_mask() -> u64 {
    MOUSE.iter().chain(&[TABLET_POINT, TABLET_PROXIMITY]).fold(0, |mask, kind| mask | 1u64 << kind)
}

pub(crate) fn start() -> Option<effectcraft_ui_egui::PenSource> {
    let source = effectcraft_ui_egui::PenSource::default();
    match monitor::install(source.clone()) {
        Ok(()) => Some(source),
        Err(error) => {
            log::warn!("tablet pen: {error}");
            None
        }
    }
}

#[allow(unsafe_code)]
mod monitor {
    use std::cell::Cell;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask};

    use super::{RawEvent, Update, event_mask, update};

    pub(super) fn install(source: effectcraft_ui_egui::PenSource) -> Result<(), &'static str> {
        MainThreadMarker::new().ok_or("the tablet monitor installs only on the main thread")?;
        let eraser = Cell::new(false);
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit calls a local monitor's handler with the event it is about to dispatch and keeps it alive
            // while the handler runs; the reference does not outlive the call.
            let read = raw(unsafe { event.as_ref() });
            let mut end = eraser.get();
            if let Update::Set(sample) = update(&mut end, &read) {
                source.set(sample);
            }
            eraser.set(end);
            event.as_ptr()
        });
        // SAFETY: the handler returns the valid event it was given, as the monitor API requires, and AppKit copies the
        // block and keeps it while the monitor is installed.
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask(event_mask()), &handler) };
        std::mem::forget(token.ok_or("AppKit refused the tablet monitor")?);
        Ok(())
    }

    fn raw(event: &NSEvent) -> RawEvent {
        let mut raw = RawEvent { kind: event.r#type().0, ..RawEvent::default() };
        if raw.mouse() {
            raw.subtype = event.subtype().0;
        }
        if raw.point() {
            raw.pressure = event.pressure();
            let tilt = event.tilt();
            raw.tilt = [tilt.x, tilt.y];
        }
        if raw.proximity() {
            raw.device = event.pointingDeviceType().0;
            raw.entering = event.isEnteringProximity();
        }
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAG: usize = 6;
    const KEY_DOWN: usize = 10;

    fn point(kind: usize, pressure: f32, tilt: [f64; 2]) -> RawEvent {
        RawEvent { kind, subtype: SUBTYPE_TABLET_POINT, pressure, tilt, ..RawEvent::default() }
    }

    fn proximity(device: usize, entering: bool) -> RawEvent {
        RawEvent { kind: TABLET_PROXIMITY, device, entering, ..RawEvent::default() }
    }

    #[test]
    fn a_pens_drag_gives_its_pressure_and_tilt_in_radians_and_the_eraser_end_marks_its_readings() {
        let mut eraser = false;
        let third = std::f32::consts::FRAC_PI_3;
        assert_eq!(update(&mut eraser, &point(DRAG, 0.5, [0.5, -1.0])), Update::Set(Some(Pen { pressure: 0.5, tilt: [third / 2.0, -third], eraser: false })));
        assert_eq!(update(&mut eraser, &proximity(DEVICE_ERASER, true)), Update::Set(Some(Pen { pressure: 0.0, tilt: [0.0; 2], eraser: true })));
        assert_eq!(update(&mut eraser, &point(TABLET_POINT, 0.25, [4.0, 0.0])), Update::Set(Some(Pen { pressure: 0.25, tilt: [third, 0.0], eraser: true })));
        assert_eq!(update(&mut eraser, &proximity(DEVICE_ERASER, false)), Update::Set(None), "the pen left");
        assert_eq!(update(&mut eraser, &point(DRAG, 0.75, [0.0, 0.0])), Update::Set(Some(Pen { pressure: 0.75, tilt: [0.0; 2], eraser: false })));
    }

    #[test]
    fn a_mouse_or_a_puck_has_no_pen_and_other_events_change_nothing() {
        let mut eraser = false;
        assert_eq!(update(&mut eraser, &RawEvent { kind: DRAG, pressure: 1.0, ..RawEvent::default() }), Update::Set(None));
        assert_eq!(update(&mut eraser, &proximity(DEVICE_CURSOR, true)), Update::Set(None), "a puck");
        assert_eq!(update(&mut eraser, &RawEvent { kind: KEY_DOWN, subtype: SUBTYPE_TABLET_POINT, ..RawEvent::default() }), Update::Keep);
        assert_eq!(event_mask() & (1 << KEY_DOWN), 0);
        assert_ne!(event_mask() & (1 << TABLET_POINT), 0);
    }
}
