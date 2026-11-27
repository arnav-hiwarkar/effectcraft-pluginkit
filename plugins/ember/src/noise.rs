use crate::random::mix;

fn lattice(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let h = mix(mix(mix(seed, x as u32), y as u32), z as u32);
    (h >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

fn fade(t: f32) -> (f32, f32) {
    (t * t * t * (t * (t * 6.0 - 15.0) + 10.0), 30.0 * t * t * (t * (t - 2.0) + 1.0))
}

pub fn value_with_gradient(p: [f32; 3], seed: u32) -> [f32; 4] {
    let cell = p.map(|v| v.floor());
    let [ix, iy, iz] = cell.map(|v| v as i32);
    let [(u, du), (v, dv), (w, dw)] = [0, 1, 2].map(|a| fade(p[a] - cell[a]));
    let c = |dx: i32, dy: i32, dz: i32| lattice(ix + dx, iy + dy, iz + dz, seed);
    let (a, b, cc, d) = (c(0, 0, 0), c(1, 0, 0), c(0, 1, 0), c(1, 1, 0));
    let (e, f, g, h) = (c(0, 0, 1), c(1, 0, 1), c(0, 1, 1), c(1, 1, 1));
    let k1 = b - a;
    let k2 = cc - a;
    let k3 = e - a;
    let k4 = a - b - cc + d;
    let k5 = a - cc - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + cc - d + e - f - g + h;
    let value = a + k1 * u + k2 * v + k3 * w + k4 * u * v + k5 * v * w + k6 * w * u + k7 * u * v * w;
    let gx = du * (k1 + k4 * v + k6 * w + k7 * v * w);
    let gy = dv * (k2 + k5 * w + k4 * u + k7 * w * u);
    let gz = dw * (k3 + k6 * u + k5 * v + k7 * u * v);
    [value, gx, gy, gz]
}

pub fn curl(p: [f32; 3], seed: u32) -> [f32; 3] {
    let a = value_with_gradient(p, mix(seed, 1));
    let b = value_with_gradient([p[0] + 31.4, p[1] - 17.1, p[2] + 9.7], mix(seed, 2));
    let c = value_with_gradient([p[0] - 23.9, p[1] + 41.3, p[2] - 5.3], mix(seed, 3));
    [c[2] - b[3], a[3] - c[1], b[1] - a[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analytic_gradient_matches_finite_differences() {
        let p = [1.37, -2.71, 0.42];
        let g = value_with_gradient(p, 9);
        let eps = 1e-3;
        for axis in 0..3 {
            let mut hi = p;
            let mut lo = p;
            hi[axis] += eps;
            lo[axis] -= eps;
            let numeric = (value_with_gradient(hi, 9)[0] - value_with_gradient(lo, 9)[0]) / (2.0 * eps);
            assert!((numeric - g[axis + 1]).abs() < 0.01, "axis {axis}: {numeric} vs {}", g[axis + 1]);
        }
    }

    #[test]
    fn the_curl_field_has_no_divergence() {
        let eps = 1e-3;
        for p in [[0.3, 0.6, 0.1], [4.2, -1.5, 2.25], [-7.7, 3.3, -0.9]] {
            let divergence: f32 = (0..3)
                .map(|axis| {
                    let mut hi = p;
                    let mut lo = p;
                    hi[axis] += eps;
                    lo[axis] -= eps;
                    (curl(hi, 5)[axis] - curl(lo, 5)[axis]) / (2.0 * eps)
                })
                .sum();
            assert!(divergence.abs() < 0.02, "divergence {divergence} at {p:?}");
        }
        assert!(curl([0.3, 0.6, 0.1], 5).iter().any(|c| c.abs() > 0.01));
    }
}
