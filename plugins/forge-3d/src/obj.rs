use crate::math::Vec3;
use crate::mesh::Mesh;

fn numbers<const N: usize>(fields: &[&str]) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    for (slot, field) in out.iter_mut().zip(fields) {
        *slot = field.parse().ok()?;
    }
    (fields.len() >= N).then_some(out)
}

fn resolve(field: Option<&str>, count: usize) -> Option<usize> {
    let index: i64 = field.filter(|text| !text.is_empty())?.parse().ok()?;
    let at = if index < 0 { count as i64 + index } else { index - 1 };
    (0..count as i64).contains(&at).then_some(at as usize)
}

pub fn parse(text: &str) -> Result<Mesh, String> {
    let (mut positions, mut normals, mut uvs) = (Vec::<Vec3>::new(), Vec::<Vec3>::new(), Vec::<[f32; 2]>::new());
    let mut mesh = Mesh::default();
    for (number, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        let bad = || format!("line {}: {line:?} is not a valid OBJ statement", number + 1);
        match fields.split_first() {
            Some((&"v", rest)) => positions.push(numbers(rest).ok_or_else(bad)?),
            Some((&"vn", rest)) => normals.push(numbers(rest).ok_or_else(bad)?),
            Some((&"vt", rest)) => uvs.push(numbers(rest).ok_or_else(bad)?),
            Some((&"f", rest)) => {
                let mut corners = Vec::with_capacity(rest.len());
                for corner in rest {
                    let mut parts = corner.split('/');
                    let position = resolve(parts.next(), positions.len()).ok_or_else(bad)?;
                    let uv = resolve(parts.next(), uvs.len());
                    let normal = resolve(parts.next(), normals.len());
                    corners.push((positions[position], uv.map(|at| uvs[at]), normal.map(|at| normals[at])));
                }
                if corners.len() < 3 {
                    return Err(bad());
                }
                if corners.iter().all(|(_, _, normal)| normal.is_some()) {
                    let first = mesh.vertex_count() as u32;
                    for (position, uv, normal) in &corners {
                        mesh.push(*position, crate::math::normalize(normal.unwrap_or_default()), uv.unwrap_or_default());
                    }
                    for i in 1..corners.len() as u32 - 1 {
                        mesh.indices.extend([first, first + i, first + i + 1]);
                    }
                } else {
                    let flat: Vec<Vec3> = corners.iter().map(|(position, _, _)| *position).collect();
                    mesh.flat(&flat);
                }
            }
            _ => {}
        }
    }
    if mesh.indices.is_empty() {
        return Err("the OBJ file has no faces".into());
    }
    mesh.fit_unit_sphere();
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_quad_without_normals_becomes_two_flat_triangles() {
        let mesh = parse("# a square\nv 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nf 1 2 3 4\n").unwrap();
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.normal(0), [0.0, 0.0, 1.0]);
        assert!((mesh.position(2)[0] - 0.5f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn corners_may_carry_uvs_normals_and_negative_indices() {
        let mesh = parse("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.25 0.75\nvn 0 0 -2\nf -3/1/1 -2/1/1 -1/1/1\n").unwrap();
        assert_eq!(mesh.normal(1), [0.0, 0.0, -1.0]);
        assert_eq!(&mesh.vertices[6..8], &[0.25, 0.75]);
    }

    #[test]
    fn a_face_naming_a_missing_vertex_is_an_error() {
        assert_eq!(parse("v 0 0 0\nf 1 2 3\n"), Err("line 2: \"f 1 2 3\" is not a valid OBJ statement".into()));
        assert_eq!(parse("v 0 0 0\n"), Err("the OBJ file has no faces".into()));
    }

    #[test]
    fn the_bundled_gem_parses() {
        let mesh = parse(include_str!("../assets/gem.obj")).unwrap();
        assert_eq!((mesh.vertex_count(), mesh.indices.len()), (180, 180));
    }
}
