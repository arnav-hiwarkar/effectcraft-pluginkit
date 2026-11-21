#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowLayer {
    pub radius: f32,
    pub weight: f32,
    pub heat: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Distortion {
    pub amplitude: f32,
    pub wavelength: f32,
    pub speed: f32,
    pub flow: f32,
    pub octaves: u32,
    pub roughness: f32,
    pub core: f32,
    pub glow: f32,
    pub glow_width: f32,
    pub core_width: f32,
    pub pin: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flicker {
    pub amount: f32,
    pub rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub colour: [f32; 3],
    pub inner_hue: f32,
    pub outer_hue: f32,
    pub inner_white: f32,
    pub core_tint: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub name: &'static str,
    pub palette: Palette,
    pub core_gain: f32,
    pub core_width: f32,
    pub glow: [GlowLayer; 5],
    pub distortion: Distortion,
    pub flicker: Flicker,
    pub strands: u32,
}

const fn layers(table: [[f32; 3]; 5]) -> [GlowLayer; 5] {
    let mut out = [GlowLayer { radius: 0.0, weight: 0.0, heat: 0.0 }; 5];
    let mut i = 0;
    while i < 5 {
        out[i] = GlowLayer { radius: table[i][0], weight: table[i][1], heat: table[i][2] };
        i += 1;
    }
    out
}

const CALM: Distortion = Distortion {
    amplitude: 1.5,
    wavelength: 120.0,
    speed: 0.6,
    flow: 0.0,
    octaves: 3,
    roughness: 0.5,
    core: 0.6,
    glow: 1.0,
    glow_width: 0.15,
    core_width: 0.05,
    pin: 0.0,
};

pub const LOOKS: [Look; 8] = [
    Look {
        name: "Default",
        palette: Palette { colour: [0.15, 0.45, 1.0], inner_hue: 0.0, outer_hue: 0.0, inner_white: 0.35, core_tint: 0.05 },
        core_gain: 1.6,
        core_width: 1.0,
        glow: layers([[1.5, 13.2, 1.0], [4.0, 4.4, 0.8], [12.0, 1.76, 0.3], [32.0, 1.32, 0.0], [90.0, 0.6, 0.0]]),
        distortion: CALM,
        strands: 1,
        flicker: Flicker { amount: 0.05, rate: 4.0 },
    },
    Look {
        name: "Neon",
        palette: Palette { colour: [1.0, 0.12, 0.6], inner_hue: 0.0, outer_hue: -10.0, inner_white: 0.2, core_tint: 0.45 },
        core_gain: 1.3,
        core_width: 1.4,
        glow: layers([[2.0, 6.6, 0.9], [6.0, 3.3, 0.5], [16.0, 1.98, 0.1], [45.0, 1.65, 0.0], [120.0, 0.83, 0.0]]),
        distortion: Distortion { amplitude: 0.3, wavelength: 200.0, speed: 0.2, octaves: 2, glow_width: 0.05, core_width: 0.0, ..CALM },
        strands: 1,
        flicker: Flicker { amount: 0.12, rate: 9.0 },
    },
    Look {
        name: "Electric",
        palette: Palette { colour: [0.25, 0.45, 1.0], inner_hue: 0.0, outer_hue: 8.0, inner_white: 0.25, core_tint: 0.0 },
        core_gain: 2.0,
        core_width: 0.45,
        glow: layers([[1.2, 3.0, 1.0], [3.5, 1.2, 0.5], [10.0, 0.8, 0.1], [30.0, 0.8, 0.0], [80.0, 0.49, 0.0]]),
        distortion: Distortion {
            amplitude: 60.0,
            wavelength: 420.0,
            speed: 3.0,
            flow: 0.0,
            octaves: 7,
            roughness: 0.68,
            core: 1.0,
            glow: 0.85,
            glow_width: 0.4,
            core_width: 0.35,
            pin: 1.0,
        },
        strands: 3,
        flicker: Flicker { amount: 0.45, rate: 14.0 },
    },
    Look {
        name: "Fire",
        palette: Palette { colour: [1.0, 0.35, 0.06], inner_hue: 22.0, outer_hue: -12.0, inner_white: 0.25, core_tint: 0.35 },
        core_gain: 1.5,
        core_width: 1.0,
        glow: layers([[2.0, 7.7, 1.0], [6.0, 3.96, 0.8], [16.0, 2.2, 0.4], [40.0, 1.65, 0.0], [100.0, 0.66, 0.0]]),
        distortion: Distortion {
            amplitude: 6.0,
            wavelength: 70.0,
            speed: 1.6,
            flow: 2.0,
            octaves: 4,
            roughness: 0.55,
            core: 0.5,
            glow: 1.6,
            glow_width: 0.7,
            core_width: 0.3,
            pin: 0.0,
        },
        strands: 1,
        flicker: Flicker { amount: 0.18, rate: 7.0 },
    },
    Look {
        name: "Plasma",
        palette: Palette { colour: [0.75, 0.2, 1.0], inner_hue: 40.0, outer_hue: -30.0, inner_white: 0.3, core_tint: 0.15 },
        core_gain: 1.7,
        core_width: 1.1,
        glow: layers([[2.0, 8.8, 1.0], [6.0, 4.4, 0.7], [18.0, 2.2, 0.3], [48.0, 1.81, 0.0], [130.0, 0.83, 0.0]]),
        distortion: Distortion {
            amplitude: 10.0,
            wavelength: 180.0,
            speed: 0.9,
            flow: 1.2,
            octaves: 3,
            roughness: 0.45,
            core: 0.8,
            glow: 1.2,
            glow_width: 0.5,
            core_width: 0.15,
            pin: 0.3,
        },
        strands: 1,
        flicker: Flicker { amount: 0.08, rate: 3.0 },
    },
    Look {
        name: "Portal",
        palette: Palette { colour: [0.1, 0.8, 1.0], inner_hue: -15.0, outer_hue: 35.0, inner_white: 0.4, core_tint: 0.1 },
        core_gain: 1.8,
        core_width: 1.5,
        glow: layers([[3.0, 6.6, 1.0], [9.0, 4.4, 0.6], [24.0, 2.64, 0.2], [60.0, 2.31, 0.0], [150.0, 1.16, 0.0]]),
        distortion: Distortion {
            amplitude: 5.0,
            wavelength: 60.0,
            speed: 1.2,
            flow: 3.0,
            octaves: 4,
            roughness: 0.6,
            core: 0.4,
            glow: 1.4,
            glow_width: 0.6,
            core_width: 0.2,
            pin: 0.0,
        },
        strands: 1,
        flicker: Flicker { amount: 0.1, rate: 5.0 },
    },
    Look {
        name: "Blade",
        palette: Palette { colour: [0.2, 1.0, 0.25], inner_hue: 0.0, outer_hue: 0.0, inner_white: 0.5, core_tint: 0.0 },
        core_gain: 2.2,
        core_width: 1.6,
        glow: layers([[2.0, 13.2, 1.0], [5.0, 5.5, 0.8], [12.0, 2.64, 0.3], [30.0, 1.65, 0.0], [70.0, 0.66, 0.0]]),
        distortion: Distortion {
            amplitude: 0.8,
            wavelength: 90.0,
            speed: 1.0,
            octaves: 2,
            core: 0.3,
            glow_width: 0.12,
            core_width: 0.04,
            pin: 0.5,
            ..CALM
        },
        strands: 1,
        flicker: Flicker { amount: 0.06, rate: 12.0 },
    },
    Look {
        name: "Laser",
        palette: Palette { colour: [1.0, 0.08, 0.05], inner_hue: 0.0, outer_hue: 0.0, inner_white: 0.3, core_tint: 0.1 },
        core_gain: 1.8,
        core_width: 0.5,
        glow: layers([[1.0, 8.8, 1.0], [3.0, 3.3, 0.6], [9.0, 1.32, 0.1], [25.0, 0.99, 0.0], [70.0, 0.39, 0.0]]),
        distortion: Distortion { amplitude: 0.3, speed: 0.3, octaves: 2, glow_width: 0.05, core_width: 0.0, ..CALM },
        strands: 1,
        flicker: Flicker { amount: 0.03, rate: 20.0 },
    },
];

pub fn names() -> [&'static str; LOOKS.len()] {
    LOOKS.map(|look| look.name)
}
