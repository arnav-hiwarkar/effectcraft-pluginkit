use std::sync::OnceLock;

use craft_plugin::Pixel;

const STEPS: usize = 4096;
const BRIGHT_LIMIT: f32 = 17.0;

pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.040_45 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

struct Table {
    start: f32,
    steps_per_unit: f32,
    values: Vec<f32>,
}

impl Table {
    fn sampled(curve: fn(f32) -> f32, start: f32, end: f32) -> Table {
        let steps_per_unit = STEPS as f32 / (end - start);
        let values = (0..=STEPS)
            .map(|i| curve(start + i as f32 / steps_per_unit))
            .collect();
        Table {
            start,
            steps_per_unit,
            values,
        }
    }

    fn lookup(&self, x: f32) -> f32 {
        let t = (x - self.start) * self.steps_per_unit;
        let i = (t as usize).min(STEPS - 1);
        let f = t - i as f32;
        let (a, b) = (self.values[i], self.values[i + 1]);
        a + (b - a) * f
    }
}

pub struct Curve {
    exact: fn(f32) -> f32,
    unit: Table,
    bright: Table,
}

impl Curve {
    fn new(exact: fn(f32) -> f32) -> Curve {
        Curve {
            exact,
            unit: Table::sampled(exact, 0.0, 1.0),
            bright: Table::sampled(exact, 1.0, BRIGHT_LIMIT),
        }
    }

    fn eval(&self, x: f32) -> f32 {
        if (0.0..1.0).contains(&x) {
            self.unit.lookup(x)
        } else if (1.0..BRIGHT_LIMIT).contains(&x) {
            self.bright.lookup(x)
        } else {
            (self.exact)(x)
        }
    }

    fn apply(&self, p: Pixel) -> Pixel {
        [self.eval(p[0]), self.eval(p[1]), self.eval(p[2]), p[3]]
    }
}

pub struct Curves {
    decode: Curve,
    encode: Curve,
}

fn curves() -> &'static Curves {
    static CURVES: OnceLock<Curves> = OnceLock::new();
    CURVES.get_or_init(|| Curves {
        decode: Curve::new(srgb_to_linear),
        encode: Curve::new(linear_to_srgb),
    })
}

#[derive(Clone, Copy)]
pub enum Transfer {
    Linear,
    Srgb(&'static Curves),
}

impl Transfer {
    pub fn srgb() -> Transfer {
        Transfer::Srgb(curves())
    }

    pub fn is_srgb(self) -> bool {
        matches!(self, Transfer::Srgb(_))
    }

    pub fn decode(self, p: Pixel) -> Pixel {
        match self {
            Transfer::Linear => p,
            Transfer::Srgb(c) => c.decode.apply(p),
        }
    }

    pub fn encode(self, p: Pixel) -> Pixel {
        match self {
            Transfer::Linear => p,
            Transfer::Srgb(c) => c.encode.apply(p),
        }
    }

    pub fn decode_value(self, x: f32) -> f32 {
        match self {
            Transfer::Linear => x,
            Transfer::Srgb(_) => srgb_to_linear(x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trip_stays_within_a_tenth_of_an_eight_bit_step() {
        for i in 0..=4000 {
            let x = i as f32 / 200.0;
            let back = Transfer::srgb().encode(Transfer::srgb().decode([x, x, x, 1.0]))[0];
            assert!((back - x).abs() < 0.1 / 255.0 * x.max(1.0), "{x} -> {back}");
        }
    }

    #[test]
    fn table_decode_matches_the_standard_curve() {
        for i in 0..=1600 {
            let x = i as f32 / 100.0;
            let (table, exact) = (Transfer::srgb().decode([x; 4])[0], srgb_to_linear(x));
            assert!(
                (table - exact).abs() <= 1e-4 * exact.max(1e-2),
                "{x}: {table} vs {exact}"
            );
        }
        assert!((Transfer::srgb().decode([0.5; 4])[0] - 0.214_041).abs() < 1e-5);
    }
}
