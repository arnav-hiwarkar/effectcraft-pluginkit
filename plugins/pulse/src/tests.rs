use std::f64::consts::PI;

use crate::dsp::{WINDOW, band_energy, fft, mono, spectrum, window_at};

fn tone(freq: f64, rate: f64, n: usize) -> Vec<f32> {
    (0..n).map(|i| (2.0 * PI * freq * i as f64 / rate).sin() as f32).collect()
}

#[test]
fn the_fft_of_an_impulse_is_flat() {
    let mut re = vec![0.0; 8];
    let mut im = vec![0.0; 8];
    re[0] = 1.0;
    fft(&mut re, &mut im);
    assert_eq!(re, vec![1.0; 8]);
    assert!(im.iter().all(|v| v.abs() < 1e-12));
}

#[test]
fn a_tone_peaks_in_the_bin_of_its_frequency_with_its_amplitude() {
    let rate = 48_000.0;
    let bin_hz = rate / WINDOW as f64;
    let freq = 40.0 * bin_hz;
    let found = spectrum(&tone(freq, rate, WINDOW));
    let peak = (0..found.len()).max_by(|a, b| found[*a].total_cmp(&found[*b])).unwrap();
    assert_eq!(peak, 40);
    assert!((found[40] - 1.0).abs() < 1e-6, "{}", found[40]);
}

#[test]
fn band_energy_sees_a_tone_inside_the_band_and_not_outside_it() {
    let rate = 48_000.0;
    let bass = spectrum(&tone(100.0, rate, WINDOW));
    let inside = band_energy(&bass, rate, 60.0, 250.0);
    let outside = band_energy(&bass, rate, 2000.0, 4000.0);
    assert!(inside > 0.9 && outside < 0.01 * inside, "inside {inside} outside {outside}");
}

#[test]
fn windows_past_the_ends_read_silence_and_stereo_becomes_mono() {
    assert_eq!(mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    let w = window_at(&[1.0; 4], 0);
    assert_eq!((w.len(), w[WINDOW / 2 - 1], w[WINDOW / 2], w[WINDOW / 2 + 4]), (WINDOW, 0.0, 1.0, 0.0));
}
