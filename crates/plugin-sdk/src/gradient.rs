use serde::{Deserialize, Serialize};

pub const MAX_GRADIENT_STOPS: usize = 64;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Gradient {
    pub colors: Vec<(f64, [f64; 4])>,
    #[serde(default)]
    pub opacities: Vec<(f64, f64)>,
}

impl Gradient {
    pub fn new(colors: &[(f64, [f64; 4])]) -> Self {
        Gradient { colors: colors.to_vec(), opacities: vec![(0.0, 1.0), (1.0, 1.0)] }
    }

    pub fn sample(&self, t: f64) -> [f32; 4] {
        let t = t.clamp(0.0, 1.0);
        let [r, g, b, a] = stops_at(&self.colors, t, |a, b, f| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)).unwrap_or([1.0; 4]);
        let opacity = stops_at(&self.opacities, t, |a, b, f| a + (b - a) * f).unwrap_or(1.0);
        [r as f32, g as f32, b as f32, (a * opacity) as f32]
    }

    pub fn check(&self) -> Result<(), String> {
        let (colors, opacities) = (self.colors.len(), self.opacities.len());
        if colors == 0 || colors > MAX_GRADIENT_STOPS || opacities > MAX_GRADIENT_STOPS {
            return Err(format!("has {colors} colour and {opacities} opacity stops; give 1 to {MAX_GRADIENT_STOPS} colour stops and at most {MAX_GRADIENT_STOPS} opacity stops"));
        }
        let positions = self.colors.iter().map(|stop| stop.0).chain(self.opacities.iter().map(|stop| stop.0));
        if !positions.into_iter().all(|at| (0.0..=1.0).contains(&at)) || !self.colors.iter().all(|stop| stop.1.iter().all(|c| c.is_finite())) {
            return Err("has a stop outside 0..=1 or a colour that is not a finite number".into());
        }
        Ok(())
    }

    pub(crate) fn limited(mut self) -> Self {
        self.colors.truncate(MAX_GRADIENT_STOPS);
        self.opacities.truncate(MAX_GRADIENT_STOPS);
        self
    }
}

fn stops_at<T: Copy>(stops: &[(f64, T)], t: f64, lerp: impl Fn(T, T, f64) -> T) -> Option<T> {
    let first = stops.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for pair in stops.windows(2) {
        if t <= pair[1].0 {
            let span = (pair[1].0 - pair[0].0).max(1e-12);
            return Some(lerp(pair[0].1, pair[1].1, (t - pair[0].0) / span));
        }
    }
    stops.last().map(|stop| stop.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gradient_samples_as_the_app_samples_its_gradients() {
        let gradient = Gradient { colors: vec![(0.0, [1.0, 0.0, 0.0, 0.8]), (1.0, [0.0, 0.0, 1.0, 0.4])], opacities: vec![(0.0, 0.5), (1.0, 0.5)] };
        assert_eq!(gradient.sample(0.5), [0.5, 0.0, 0.5, 0.3]);
        assert_eq!(gradient.sample(-1.0), [1.0, 0.0, 0.0, 0.4]);
        let ramp = Gradient::new(&[(0.25, [0.0, 0.0, 0.0, 1.0]), (0.75, [1.0, 1.0, 1.0, 1.0])]);
        assert_eq!([ramp.sample(0.1), ramp.sample(0.5), ramp.sample(0.9)], [[0.0, 0.0, 0.0, 1.0], [0.5, 0.5, 0.5, 1.0], [1.0; 4]]);
        assert_eq!(Gradient::default().sample(0.3), [1.0; 4]);
    }

    #[test]
    fn a_gradient_with_more_stops_than_the_limit_is_refused_with_the_limit_named() {
        let spec = |count: usize| {
            let stops: Vec<(f64, [f64; 4])> = (0..count).map(|i| (i as f64 / count as f64, [0.0, 0.0, 0.0, 1.0])).collect();
            crate::EffectSpec::new("ramp", "Ramp", "Probe").param(crate::Param::gradient("ramp", "Ramp", &stops))
        };
        assert_eq!(spec(MAX_GRADIENT_STOPS).check(), Ok(()));
        let refused = "effect `ramp`: gradient `ramp` has 65 colour and 2 opacity stops; give 1 to 64 colour stops and at most 64 opacity stops";
        assert_eq!(spec(MAX_GRADIENT_STOPS + 1).check(), Err(refused.to_string()));
    }
}
