pub const REACH_IN_SIGMAS: f32 = 2.5;
pub const TAIL_IN_SIGMAS: f32 = 3.5;
pub const MAX_OCTAVES: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub radius: f32,
    pub falloff: f32,
}

impl Profile {
    pub fn tail_power(&self) -> f32 {
        3.0 - 3.0 * self.falloff.clamp(0.0, 1.0)
    }

    pub fn reach_octave(radius: f32) -> f32 {
        (radius.max(1e-3) / REACH_IN_SIGMAS).log2()
    }

    pub fn tail_extent(radius: f32) -> f32 {
        radius / REACH_IN_SIGMAS * TAIL_IN_SIGMAS
    }

    pub fn octave_count(radius: f32) -> usize {
        ((Profile::reach_octave(radius) + 0.5).ceil().max(1.0) as usize).min(MAX_OCTAVES)
    }

    pub fn weights(&self, octaves: usize) -> Vec<f32> {
        let reach = Profile::reach_octave(self.radius);
        let growth = 2.0 - self.tail_power();
        let raw: Vec<f32> = (0..octaves)
            .map(|i| (i as f32 * growth).exp2() * (0.5 + reach - i as f32).clamp(0.0, 1.0))
            .collect();
        let total: f32 = raw.iter().sum();
        if total <= 0.0 {
            return (0..octaves)
                .map(|i| if i == 0 { 1.0 } else { 0.0 })
                .collect();
        }
        raw.iter().map(|w| w / total).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide_share(w: &[f32]) -> f32 {
        w[w.len() / 2..].iter().sum()
    }

    #[test]
    fn weights_conserve_energy() {
        for radius in [1.0, 10.0, 100.0, 1000.0] {
            let p = Profile {
                radius,
                falloff: 0.5,
            };
            let total: f32 = p.weights(Profile::octave_count(radius)).iter().sum();
            assert!((total - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn falloff_moves_energy_into_the_wide_octaves() {
        let tight = Profile {
            radius: 300.0,
            falloff: 0.0,
        }
        .weights(7);
        let wide = Profile {
            radius: 300.0,
            falloff: 1.0,
        }
        .weights(7);
        assert!(wide_share(&wide) > 0.95 && wide_share(&tight) < 0.15);
    }

    #[test]
    fn neutral_falloff_spreads_energy_evenly_per_octave() {
        let w = Profile {
            radius: REACH_IN_SIGMAS * 32.0 * std::f32::consts::SQRT_2,
            falloff: 1.0 / 3.0,
        }
        .weights(6);
        assert!(w.iter().all(|x| (x - 1.0 / 6.0).abs() < 1e-6), "{w:?}");
    }

    #[test]
    fn weights_change_continuously_with_radius() {
        let at = |r: f32| {
            Profile {
                radius: r,
                falloff: 0.5,
            }
            .weights(MAX_OCTAVES)
        };
        for i in 1..2000 {
            let r = i as f32 * 0.5;
            let (a, b) = (at(r), at(r + 0.01));
            assert!(
                a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.01),
                "jump at radius {r}"
            );
        }
    }

    #[test]
    fn octave_count_grows_with_radius() {
        assert_eq!([3.0, 100.0, 1000.0].map(Profile::octave_count), [1, 6, 10]);
    }
}
