use craft_plugin::Pixel;

use crate::image::Image;
use crate::pixel::{CLEAR, mul_add};

const SIGMA_SPAN: f32 = 2.5;
const NEGLIGIBLE_SIGMA: f32 = 0.2;
const THREE_TAP_MAX_VARIANCE: f32 = 2.0 / 3.0;

pub fn gaussian(sigma: f32) -> Vec<f32> {
    let variance = sigma * sigma;
    if variance <= THREE_TAP_MAX_VARIANCE {
        let side = variance / 2.0;
        return vec![side, 1.0 - 2.0 * side, side];
    }
    let radius = (sigma * SIGMA_SPAN).ceil() as isize;
    let raw: Vec<f32> = (-radius..=radius)
        .map(|x| (-((x * x) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = raw.iter().sum();
    raw.iter().map(|w| w / total).collect()
}

pub fn blur(image: &mut Image, sigma: [f32; 2], padded: &mut Vec<Pixel>, copy: &mut Vec<Pixel>) {
    if sigma[0] > NEGLIGIBLE_SIGMA {
        blur_rows(image, &gaussian(sigma[0]), padded);
    }
    if sigma[1] > NEGLIGIBLE_SIGMA {
        blur_columns(image, &gaussian(sigma[1]), copy);
    }
}

fn blur_rows(image: &mut Image, kernel: &[f32], padded: &mut Vec<Pixel>) {
    let radius = kernel.len() / 2;
    let width = image.width;
    padded.clear();
    padded.resize(width + 2 * radius, CLEAR);
    for y in 0..image.height {
        let row = image.row_mut(y);
        padded[radius..radius + width].copy_from_slice(row);
        row.fill(CLEAR);
        for (k, w) in kernel.iter().enumerate() {
            row.iter_mut()
                .zip(&padded[k..k + width])
                .for_each(|(o, p)| *o = mul_add(*o, *p, *w));
        }
    }
}

fn blur_columns(image: &mut Image, kernel: &[f32], copy: &mut Vec<Pixel>) {
    let radius = kernel.len() as isize / 2;
    copy.clear();
    copy.extend_from_slice(&image.pixels);
    let source = Image {
        width: image.width,
        height: image.height,
        pixels: std::mem::take(copy),
    };
    for y in 0..image.height {
        let out = image.row_mut(y);
        out.fill(CLEAR);
        for (k, w) in kernel.iter().enumerate() {
            if let Some(row) = source.row_at(y as isize + k as isize - radius) {
                out.iter_mut()
                    .zip(row)
                    .for_each(|(o, p)| *o = mul_add(*o, *p, *w));
            }
        }
    }
    *copy = source.pixels;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_is_normalised_and_symmetric() {
        let k = gaussian(1.5);
        assert_eq!(k.len(), 9);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(k.iter().zip(k.iter().rev()).all(|(a, b)| a == b));
    }

    #[test]
    fn small_kernels_have_exactly_the_requested_variance() {
        for sigma in [0.3f32, 0.6, 0.8] {
            let k = gaussian(sigma);
            let r = (k.len() / 2) as f32;
            let variance: f32 = k
                .iter()
                .enumerate()
                .map(|(i, w)| w * (i as f32 - r).powi(2))
                .sum();
            assert!(
                (variance - sigma * sigma).abs() < 1e-6,
                "{sigma}: {variance}"
            );
        }
    }

    #[test]
    fn blur_keeps_energy_of_an_interior_point() {
        let mut image = Image::new([21, 21]);
        image.pixels[10 * 21 + 10] = [1.0; 4];
        blur(&mut image, [1.0, 2.0], &mut Vec::new(), &mut Vec::new());
        let energy: f32 = image.pixels.iter().map(|p| p[0]).sum();
        assert!((energy - 1.0).abs() < 1e-5);
        assert!(image.pixels[10 * 21 + 13][0] < image.pixels[13 * 21 + 10][0]);
    }
}
