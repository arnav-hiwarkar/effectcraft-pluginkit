use craft_plugin::Frame;

use crate::draw::{BIN_ROWS, DrawList, SPARK_LENGTH, Sprite};
use crate::settings::Shape;

const SPARK_THINNING: f32 = SPARK_LENGTH * SPARK_LENGTH;

fn coverage(shape: Shape, sprite: &Sprite, (sin, cos): (f32, f32), centre: [f32; 2], x: f32, y: f32) -> f32 {
    let (dx, dy) = (x - centre[0], y - centre[1]);
    let r2 = sprite.radius * sprite.radius;
    let d2 = match shape {
        Shape::Disc => dx * dx + dy * dy,
        Shape::Streak => {
            let [sx, sy] = sprite.streak;
            let len2 = sx * sx + sy * sy;
            let t = if len2 > 0.0 { ((dx * sx + dy * sy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (dx - sx * t, dy - sy * t);
            ex * ex + ey * ey
        }
        Shape::Spark => {
            let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            u * u + v * v * SPARK_THINNING * SPARK_THINNING
        }
    };
    let k = 1.0 - d2 / r2;
    if k > 0.0 { k * k } else { 0.0 }
}

fn draw(list: &DrawList, sprite: &Sprite, layer: &mut [[f32; 4]], width: usize, rows: (usize, usize), row_base: usize) {
    let samples = list.samples.max(1);
    let weight = 1.0 / samples as f32;
    let trig = sprite.angle.sin_cos();
    let (left, right) = sprite.columns();
    let x0 = left.max(0.0) as usize;
    let x1 = (right.ceil().max(0.0) as usize).min(width);
    let (top, bottom) = sprite.rows();
    let y0 = (top.max(0.0) as usize).max(rows.0);
    let y1 = (bottom.ceil().max(0.0) as usize).min(rows.1);
    for y in y0..y1 {
        let row = &mut layer[(y - row_base) * width..(y - row_base + 1) * width];
        let py = y as f32 + 0.5;
        for (x, pixel) in row.iter_mut().enumerate().take(x1).skip(x0) {
            let px = x as f32 + 0.5;
            let mut cover = 0.0;
            for s in 0..samples {
                let t = if samples > 1 { s as f32 / (samples - 1) as f32 } else { 0.0 };
                let centre = [sprite.centre[0] + sprite.blur[0] * t, sprite.centre[1] + sprite.blur[1] * t];
                cover += coverage(list.shape, sprite, trig, centre, px, py);
            }
            let cover = cover * weight;
            if cover <= 0.0 {
                continue;
            }
            let c = sprite.colour.map(|v| v * cover);
            if list.additive {
                for i in 0..4 {
                    pixel[i] += c[i];
                }
            } else {
                let keep = 1.0 - c[3];
                for i in 0..4 {
                    pixel[i] = c[i] + pixel[i] * keep;
                }
            }
        }
    }
}

pub fn render(list: &DrawList, frame: &mut Frame) {
    let width = frame.width;
    let first = frame.origin[1];
    let last = first + frame.height;
    let mut layer = vec![[0.0f32; 4]; width * frame.height];
    let bins = list.bin_starts.len().saturating_sub(1);
    let first_bin = first / BIN_ROWS;
    let last_bin = last.div_ceil(BIN_ROWS).min(bins);
    for bin in first_bin..last_bin {
        let rows = ((bin * BIN_ROWS).max(first), ((bin + 1) * BIN_ROWS).min(last));
        let (start, end) = (list.bin_starts[bin] as usize, list.bin_starts[bin + 1] as usize);
        for &index in &list.bin_items[start..end] {
            draw(list, &list.sprites[index as usize], &mut layer, width, rows, first);
        }
    }
    for ((_, row), particles) in frame.rows_mut().zip(layer.chunks_exact(width.max(1))) {
        for (pixel, p) in row.iter_mut().zip(particles) {
            let alpha = p[3].min(1.0);
            if list.additive {
                *pixel = [pixel[0] + p[0], pixel[1] + p[1], pixel[2] + p[2], pixel[3] + alpha * (1.0 - pixel[3])];
            } else {
                let keep = 1.0 - alpha;
                *pixel = [p[0] + pixel[0] * keep, p[1] + pixel[1] * keep, p[2] + pixel[2] * keep, alpha + pixel[3] * keep];
            }
        }
    }
}
