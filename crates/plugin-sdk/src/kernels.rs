use serde::{Deserialize, Serialize};

use crate::wire::Pixel;

pub const MAX_BLUR_RADIUS: f32 = 512.0;
pub const MAX_TAPS: usize = 1025;
pub const MAX_FULL_SIDE: usize = 31;
pub const MAX_CURVE_POINTS: usize = 4096;
pub const MAX_RESAMPLE_SCALE: f32 = 64.0;
pub const MAX_WARP: f32 = 4096.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HostKernel {
    Blur {
        shape: BlurShape,
        radius: [f32; 2],
    },
    Resample {
        filter: Filter,
        #[serde(default)]
        scale: Option<[f32; 2]>,
    },
    Blend {
        mode: BlendMode,
        amount: f32,
    },
    Curves {
        table: Vec<Pixel>,
    },
    Convolve {
        taps: Taps,
    },
    Warp {
        amount: [f32; 2],
    },
    Matrix {
        rows: [[f32; 4]; 4],
    },
    ToLinear,
    ToSrgb,
    BrightnessCurve {
        measure: Measure,
        curve: Vec<f32>,
        range: f32,
    },
    Dither {
        seed: u32,
        origin: [i64; 2],
    },
    LightAlpha,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    MaxChannel,
    Luma,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlurShape {
    Gaussian,
    Box,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Bilinear,
    Bicubic,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Over,
    Add,
    Subtract,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Difference,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Taps {
    Separable { x: Vec<f32>, y: Vec<f32> },
    Full { width: u32, weights: Vec<f32> },
}

impl BlendMode {
    pub const ALL: [BlendMode; 9] = [
        BlendMode::Over,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
    ];
}

impl HostKernel {
    pub fn name(&self) -> &'static str {
        match self {
            HostKernel::Blur { .. } => "blur",
            HostKernel::Resample { .. } => "resample",
            HostKernel::Blend { .. } => "blend",
            HostKernel::Curves { .. } => "curves",
            HostKernel::Convolve { .. } => "convolve",
            HostKernel::Warp { .. } => "warp",
            HostKernel::Matrix { .. } => "matrix",
            HostKernel::ToLinear => "to_linear",
            HostKernel::ToSrgb => "to_srgb",
            HostKernel::BrightnessCurve { .. } => "brightness_curve",
            HostKernel::Dither { .. } => "dither",
            HostKernel::LightAlpha => "light_alpha",
        }
    }

    pub fn pointwise(&self) -> bool {
        !matches!(self, HostKernel::Blur { .. } | HostKernel::Resample { .. } | HostKernel::Convolve { .. } | HostKernel::Warp { .. })
    }

    pub(crate) fn needs_temporary(&self) -> bool {
        matches!(self, HostKernel::Blur { .. } | HostKernel::Convolve { taps: Taps::Separable { .. } })
    }

    pub fn check(&self, src: [u32; 2], aux: Option<[u32; 2]>, out: [u32; 2]) -> Result<(), String> {
        let same = |field: &str, size: [u32; 2]| {
            if size == out {
                Ok(())
            } else {
                Err(format!("`{field}` is {}x{} but `out` is {}x{}; they must be the same size", size[0], size[1], out[0], out[1]))
            }
        };
        let needs_aux = |what: &str| aux.ok_or_else(|| format!("`aux` is missing; give the {what} with `.aux(image)`"));
        match self {
            HostKernel::Blur { radius, .. } => {
                if !radius.iter().all(|r| (0.0..=MAX_BLUR_RADIUS).contains(r)) {
                    return Err(format!("`radius` is {radius:?}; each axis must be 0 to {MAX_BLUR_RADIUS}"));
                }
                same("src", src)
            }
            HostKernel::Resample { scale, .. } => match scale {
                Some(scale) if !scale.iter().all(|s| *s > 0.0 && *s <= MAX_RESAMPLE_SCALE) => {
                    Err(format!("`scale` is {scale:?}; each axis must be above 0 and at most {MAX_RESAMPLE_SCALE} source pixels per output pixel"))
                }
                None if src.iter().zip(out).any(|(s, o)| *s > o.saturating_mul(MAX_RESAMPLE_SCALE as u32)) => Err(format!(
                    "`src` is {}x{} and `out` {}x{}; one pass shrinks at most {MAX_RESAMPLE_SCALE} times, so resample in steps",
                    src[0], src[1], out[0], out[1]
                )),
                _ => Ok(()),
            },
            HostKernel::Blend { amount, .. } => {
                if !amount.is_finite() {
                    return Err(format!("`amount` is {amount}; it must be a finite number"));
                }
                same("src", src)?;
                same("aux", needs_aux("top image")?)
            }
            HostKernel::Curves { table } => {
                if !(2..=MAX_CURVE_POINTS).contains(&table.len()) {
                    return Err(format!("`table` has {} entries; it needs 2 to {MAX_CURVE_POINTS}", table.len()));
                }
                if table.iter().flatten().any(|v| !v.is_finite()) {
                    return Err("`table` holds a value that is not a finite number".into());
                }
                same("src", src)
            }
            HostKernel::Convolve { taps } => {
                taps.check()?;
                same("src", src)
            }
            HostKernel::Warp { amount } => {
                if !amount.iter().all(|a| a.abs() <= MAX_WARP) {
                    return Err(format!("`amount` is {amount:?}; each axis must be -{MAX_WARP} to {MAX_WARP} pixels"));
                }
                same("src", src)?;
                same("aux", needs_aux("displacement image")?)
            }
            HostKernel::Matrix { rows } => {
                if rows.iter().flatten().any(|v| !v.is_finite()) {
                    return Err("`rows` holds a value that is not a finite number".into());
                }
                same("src", src)
            }
            HostKernel::BrightnessCurve { curve, range, .. } => {
                if !(2..=MAX_CURVE_POINTS).contains(&curve.len()) {
                    return Err(format!("`curve` has {} entries; it needs 2 to {MAX_CURVE_POINTS}", curve.len()));
                }
                if curve.iter().any(|v| !v.is_finite()) {
                    return Err("`curve` holds a value that is not a finite number".into());
                }
                if !(range.is_finite() && *range > 0.0) {
                    return Err(format!("`range` is {range}; it must be a finite number above 0"));
                }
                same("src", src)
            }
            HostKernel::ToLinear | HostKernel::ToSrgb | HostKernel::Dither { .. } | HostKernel::LightAlpha => same("src", src),
        }
    }
}

impl Taps {
    fn check(&self) -> Result<(), String> {
        let finite = |weights: &[f32], field: &str| {
            if weights.iter().all(|w| w.is_finite()) { Ok(()) } else { Err(format!("`{field}` holds a weight that is not a finite number")) }
        };
        match self {
            Taps::Separable { x, y } => {
                for (field, taps) in [("x", x), ("y", y)] {
                    if taps.len() % 2 == 0 || taps.len() > MAX_TAPS {
                        return Err(format!("`{field}` has {} taps; give an odd number from 1 to {MAX_TAPS}, centred on the pixel", taps.len()));
                    }
                    finite(taps, field)?;
                }
                Ok(())
            }
            Taps::Full { width, weights } => {
                let width = *width as usize;
                let height = weights.len().checked_div(width).unwrap_or(0);
                let odd_side = |side: usize| side % 2 == 1 && side <= MAX_FULL_SIDE;
                if !odd_side(width) || !odd_side(height) || width * height != weights.len() {
                    return Err(format!(
                        "`weights` has {} values for `width` {width}; give an odd width and an odd number of rows, each 1 to {MAX_FULL_SIDE}",
                        weights.len()
                    ));
                }
                finite(weights, "weights")
            }
        }
    }
}
