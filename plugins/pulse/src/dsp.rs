use std::f64::consts::PI;

pub const WINDOW: usize = 2048;

pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

pub fn spectrum(mono: &[f32]) -> Vec<f64> {
    let n = mono.len();
    let mut re: Vec<f64> = mono.iter().enumerate().map(|(i, s)| f64::from(*s) * (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos())).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    let norm = 2.0 / (n as f64 * 0.5);
    (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * norm).collect()
}

pub fn band_energy(spectrum: &[f64], rate: f64, low: f64, high: f64) -> f64 {
    let bin_hz = rate / (2.0 * spectrum.len() as f64);
    let first = (low / bin_hz).floor().max(0.0) as usize;
    let last = ((high / bin_hz).ceil() as usize).min(spectrum.len().saturating_sub(1));
    (first..=last).map(|k| spectrum[k] * spectrum[k]).sum::<f64>().sqrt()
}

pub fn mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    interleaved.chunks_exact(channels.max(1)).map(|frame| frame.iter().sum::<f32>() / frame.len() as f32).collect()
}

pub fn window_at(mono: &[f32], center: isize) -> Vec<f32> {
    let half = (WINDOW / 2) as isize;
    (center - half..center + half).map(|i| if i < 0 || i as usize >= mono.len() { 0.0 } else { mono[i as usize] }).collect()
}
