use std::sync::Mutex;

use crate::curve::Curve;

static PANEL_CURVE: Mutex<Curve> = Mutex::new(Curve::DEFAULT);

pub fn publish(curve: Curve) {
    if let Ok(mut shared) = PANEL_CURVE.lock() {
        *shared = curve;
    }
}

pub fn current() -> Result<Curve, String> {
    PANEL_CURVE.lock().map(|curve| *curve).map_err(|_| "The Glide curve is unavailable; reopen the Glide panel.".to_string())
}
