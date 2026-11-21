use craft_plugin::Frame;

use crate::beam::{self, Stroke};
use crate::glow::{self, Glow};
use crate::noise;
use crate::image::{lerp_row, sample_row, taps};
use crate::path::Path;
use crate::plane::{Bounds, Plane};
use crate::settings::Settings;
use crate::shade::Shading;

fn core_plane(strokes: &[Stroke], size: [usize; 2]) -> Option<Plane> {
    let bounds = Bounds::of_nodes(strokes.iter().flat_map(|s| &s.core))?;
    let lo = [0, 1].map(|axis| (bounds.min[axis] - 2.0).floor().clamp(0.0, size[axis] as f32) as usize);
    let hi = [0, 1].map(|axis| (bounds.max[axis] + 2.0).ceil().clamp(0.0, size[axis] as f32) as usize);
    let dims = [0, 1].map(|axis| hi[axis].saturating_sub(lo[axis]));
    if dims[0] == 0 || dims[1] == 0 {
        return None;
    }
    let mut plane = Plane::new([lo[0] as i32, lo[1] as i32], dims, 1.0);
    strokes.iter().for_each(|stroke| plane.draw(&stroke.core));
    Some(plane)
}

fn composite(frame: &mut Frame, core: Option<&Plane>, glow: Option<&Glow>, shading: &Shading) {
    let mut column = vec![[0.0; 3]; glow.map_or(0, |g| g.image.width)];
    let keeps_source = shading.composite.keeps_source();
    for (y, row) in frame.rows_mut() {
        if let Some(glow) = glow {
            let [(a, wa), (b, wb)] = taps(y + glow.guard[1], glow.image.height);
            lerp_row(&mut column, glow.image.row(a), glow.image.row(b), wa, wb);
        }
        let core_row = core.and_then(|plane| plane.row_at(y));
        for (x, pixel) in row.iter_mut().enumerate() {
            let light = glow.map_or([0.0; 3], |g| sample_row(&column, x + g.guard[0]));
            let coverage = core_row
                .and_then(|(start, cells)| cells.get(x.checked_sub(start)?))
                .map_or(0.0, |cell| cell[0]);
            if keeps_source && coverage <= 0.0 && light[0] + light[1] + light[2] <= 0.0 {
                continue;
            }
            *pixel = shading.blend(*pixel, shading.beam(light, coverage));
        }
    }
}

pub fn render(settings: &Settings, paths: &[Path], frame: &mut Frame) {
    let strokes: Vec<Stroke> = paths
        .iter()
        .enumerate()
        .flat_map(|(i, path)| (0..settings.strands).map(move |strand| (path, noise::mix(i as u32, strand))))
        .map(|(path, salt)| beam::stroke(path, &settings.shape, &settings.motion, salt))
        .collect();
    let size = [frame.width, frame.height];
    let core = core_plane(&strokes, size);
    let glow = glow::render(
        strokes.iter().map(|s| s.glow.as_slice()),
        &settings.glow,
        settings.shape.glow_radius,
        settings.reach,
        &settings.shading,
        size,
    );
    composite(frame, core.as_ref(), glow.as_ref(), &settings.shading);
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_plugin::{Cx, ParamKind, RenderCx, Value};

    fn settings(overrides: &[(&str, f64)]) -> Settings {
        let size = [160.0, 90.0];
        let mut params: std::collections::BTreeMap<String, Value> = crate::spec::effect()
            .params
            .into_iter()
            .filter_map(|p| {
                let value = match p.kind {
                    ParamKind::Slider { default, .. } | ParamKind::Percent { default } => Value::Number(default),
                    ParamKind::Popup { default, .. } => Value::Number(f64::from(default)),
                    ParamKind::Point { default } => Value::List(vec![default[0] * size[0], default[1] * size[1]]),
                    ParamKind::Color { default } => Value::List(default.to_vec()),
                    _ => return None,
                };
                Some((p.id, value))
            })
            .collect();
        params.extend(overrides.iter().map(|(id, v)| (id.to_string(), Value::Number(*v))));
        let cx = RenderCx { params, time: 1.25, scale: 1.0, size: [160, 90], instance: 7, ..Default::default() };
        Settings::read(&Cx::new(cx))
    }

    fn rendered(settings: &Settings, paths: &[Path]) -> Vec<craft_plugin::Pixel> {
        let mut pixels = vec![[0.25, 0.5, 0.75, 1.0]; 160 * 90];
        let mut frame = Frame { width: 160, height: 90, origin: [0, 0], pixels: &mut pixels };
        render(settings, paths, &mut frame);
        pixels
    }

    #[test]
    fn degenerate_paths_render_finite_images_for_every_preset() {
        let paths = [
            Path::new(vec![[80.0, 45.0], [80.0, 45.0]], false),
            Path::new(vec![[30.0, 30.0]], true),
            Path::new(Vec::new(), false),
            Path::new(vec![[f32::NAN, 1.0], [10.0, 10.0]], false),
            Path::new(vec![[-5000.0, 45.0], [5000.0, 45.0]], false),
        ];
        for preset in 0..crate::look::LOOKS.len() {
            let pixels = rendered(&settings(&[("preset", preset as f64)]), &paths);
            assert!(pixels.iter().flatten().all(|v| v.is_finite()), "preset {preset}");
        }
    }

    #[test]
    fn a_zero_length_beam_still_glows_where_it_stands() {
        let pixels = rendered(&settings(&[]), &[Path::new(vec![[80.0, 45.0], [80.0, 45.0]], false)]);
        assert!(pixels[45 * 160 + 80].iter().take(3).all(|&c| c > 1.0));
    }

    #[test]
    fn on_black_replaces_the_source_everywhere() {
        let pixels = rendered(&settings(&[(crate::spec::COMPOSITE, 3.0)]), &[]);
        assert!(pixels.iter().all(|p| *p == [0.0, 0.0, 0.0, 1.0]));
    }
}
