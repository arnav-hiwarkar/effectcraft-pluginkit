use craft_plugin::Pixel;

use crate::pixel::{brightest, scale};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knee {
    threshold: f32,
    width: f32,
}

impl Knee {
    pub fn new(threshold: f32, smoothness: f32) -> Knee {
        let threshold = threshold.max(0.0);
        Knee {
            threshold,
            width: threshold * smoothness.clamp(0.0, 1.0),
        }
    }

    pub fn shape(&self) -> [f32; 2] {
        [self.threshold, self.width]
    }

    pub fn response(&self, brightness: f32) -> f32 {
        let above = brightness - self.threshold;
        if self.width <= 0.0 {
            return above.max(0.0);
        }
        let soft = (above + self.width).clamp(0.0, 2.0 * self.width);
        above.max(soft * soft / (4.0 * self.width))
    }

    pub fn emit(&self, p: Pixel) -> Pixel {
        let b = brightest(p);
        if b <= 0.0 {
            return [0.0; 4];
        }
        scale(p, self.response(b) / b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_is_monotonic_and_continuous() {
        for knee in [
            Knee::new(0.5, 0.0),
            Knee::new(0.5, 0.5),
            Knee::new(1.0, 1.0),
            Knee::new(0.0, 0.5),
        ] {
            let mut last = knee.response(0.0);
            for i in 1..=4000 {
                let r = knee.response(i as f32 / 1000.0);
                assert!(r >= last && r - last < 0.002, "{knee:?} at {i}");
                last = r;
            }
        }
    }

    #[test]
    fn soft_knee_values() {
        let knee = Knee::new(1.0, 0.5);
        assert_eq!(knee.response(0.4), 0.0);
        assert_eq!(knee.response(1.0), 0.125);
        assert_eq!(knee.response(3.0), 2.0);
    }

    #[test]
    fn zero_threshold_passes_colour_through() {
        assert_eq!(
            Knee::new(0.0, 0.5).emit([0.8, 0.2, 0.1, 1.0]),
            [0.8, 0.2, 0.1, 1.0]
        );
    }

    #[test]
    fn emission_keeps_hue_and_scales_with_hdr_brightness() {
        let knee = Knee::new(1.0, 0.0);
        assert_eq!(knee.emit([4.0, 2.0, 0.0, 1.0]), [3.0, 1.5, 0.0, 0.75]);
    }
}
