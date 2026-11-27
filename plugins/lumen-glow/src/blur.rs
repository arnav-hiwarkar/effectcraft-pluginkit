const SIGMA_SPAN: f32 = 2.5;
const NEGLIGIBLE_SIGMA: f32 = 0.2;
const THREE_TAP_MAX_VARIANCE: f32 = 2.0 / 3.0;

fn gaussian(sigma: f32) -> Vec<f32> {
    let variance = sigma * sigma;
    if variance <= THREE_TAP_MAX_VARIANCE {
        let side = variance / 2.0;
        return vec![side, 1.0 - 2.0 * side, side];
    }
    let radius = (sigma * SIGMA_SPAN).ceil() as isize;
    let raw: Vec<f32> = (-radius..=radius).map(|x| (-((x * x) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = raw.iter().sum();
    raw.iter().map(|w| w / total).collect()
}

pub fn taps(sigma: f32) -> Vec<f32> {
    if sigma > NEGLIGIBLE_SIGMA { gaussian(sigma) } else { vec![1.0] }
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
            let variance: f32 = k.iter().enumerate().map(|(i, w)| w * (i as f32 - r).powi(2)).sum();
            assert!((variance - sigma * sigma).abs() < 1e-6, "{sigma}: {variance}");
        }
    }
}
