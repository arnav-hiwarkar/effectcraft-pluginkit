pub fn mix(a: u32, b: u32) -> u32 {
    let mut x = a ^ b.wrapping_mul(0x9e37_79b9).rotate_left(13);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

pub struct Stream(u32);

impl Stream {
    pub fn new(seed: u32) -> Stream {
        Stream(mix(seed, 0x2545_f491))
    }

    pub fn next(&mut self) -> f32 {
        self.0 = mix(self.0, 0x68e3_1da4);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn signed(&mut self) -> f32 {
        2.0 * self.next() - 1.0
    }

    pub fn in_sphere(&mut self) -> [f32; 3] {
        loop {
            let p = [self.signed(), self.signed(), self.signed()];
            if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= 1.0 {
                return p;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_reproducible_and_spreads_over_the_unit_interval() {
        let draws: Vec<f32> = {
            let mut s = Stream::new(7);
            (0..1000).map(|_| s.next()).collect()
        };
        let again: Vec<f32> = {
            let mut s = Stream::new(7);
            (0..1000).map(|_| s.next()).collect()
        };
        assert_eq!(draws, again);
        assert!(draws.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = draws.iter().sum::<f32>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.03, "mean {mean}");
        assert_ne!(Stream::new(8).next(), Stream::new(7).next());
    }
}
