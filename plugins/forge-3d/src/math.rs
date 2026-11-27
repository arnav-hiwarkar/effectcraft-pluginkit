pub type Mat4 = [f32; 16];
pub type Vec3 = [f32; 3];

pub const IDENTITY: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|i| (0..4).map(|k| a[k * 4 + i % 4] * b[(i / 4) * 4 + k]).sum())
}

pub fn translate([x, y, z]: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    (m[12], m[13], m[14]) = (x, y, z);
    m
}

pub fn scale(s: f32) -> Mat4 {
    let mut m = IDENTITY;
    (m[0], m[5], m[10]) = (s, s, s);
    m
}

pub fn rotate(axis: usize, degrees: f32) -> Mat4 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut m = IDENTITY;
    m[a * 4 + a] = cos;
    m[a * 4 + b] = sin;
    m[b * 4 + a] = -sin;
    m[b * 4 + b] = cos;
    m
}

pub fn point(m: &Mat4, [x, y, z]: Vec3) -> Vec3 {
    std::array::from_fn(|i| m[i] * x + m[4 + i] * y + m[8 + i] * z + m[12 + i])
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn length(a: Vec3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l < 1e-12 { [0.0, 0.0, -1.0] } else { [a[0] / l, a[1] / l, a[2] / l] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        length(sub(a, b)) < 1e-5
    }

    #[test]
    fn a_quarter_turn_about_z_takes_x_to_y() {
        assert!(close(point(&rotate(2, 90.0), [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(close(point(&rotate(0, 90.0), [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        assert!(close(point(&rotate(1, 90.0), [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn the_right_hand_matrix_applies_first() {
        let m = mul(&translate([10.0, 0.0, 0.0]), &scale(2.0));
        assert!(close(point(&m, [1.0, 1.0, 1.0]), [12.0, 2.0, 2.0]));
    }
}
