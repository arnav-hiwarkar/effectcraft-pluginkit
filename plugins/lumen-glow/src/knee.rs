const CURVE_STEPS: usize = 4094;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knee {
    threshold: f32,
    width: f32,
}

impl Knee {
    pub fn new(threshold: f32, smoothness: f32) -> Knee {
        let threshold = threshold.max(0.0);
        Knee { threshold, width: threshold * smoothness.clamp(0.0, 1.0) }
    }

    pub fn curve(&self) -> (Vec<f32>, f32) {
        let end = self.threshold + self.width;
        if end <= 0.0 {
            return (vec![0.0, 1.0], 1.0);
        }
        let range = 2.0 * end;
        ((0..=CURVE_STEPS).map(|i| self.response(range * i as f32 / CURVE_STEPS as f32)).collect(), range)
    }

    pub fn response(&self, brightness: f32) -> f32 {
        let above = brightness - self.threshold;
        if self.width <= 0.0 {
            return above.max(0.0);
        }
        let soft = (above + self.width).clamp(0.0, 2.0 * self.width);
        above.max(soft * soft / (4.0 * self.width))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_is_monotonic_and_continuous() {
        for knee in [Knee::new(0.5, 0.0), Knee::new(0.5, 0.5), Knee::new(1.0, 1.0), Knee::new(0.0, 0.5)] {
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
    fn the_curve_follows_the_response_and_holds_the_hard_knee_on_a_sample() {
        for knee in [Knee::new(1.0, 0.0), Knee::new(1.0, 0.5), Knee::new(0.0, 0.0)] {
            let (curve, range) = knee.curve();
            let step = range / (curve.len() - 1) as f32;
            assert!(curve.iter().enumerate().all(|(i, v)| (v - knee.response(i as f32 * step)).abs() < 1e-6), "{knee:?}");
        }
        let (curve, range) = Knee::new(1.0, 0.0).curve();
        assert_eq!((curve.len(), range * 2047.0 / 4094.0), (4095, 1.0), "the hard knee at 1.0 is sample 2047");
    }
}
