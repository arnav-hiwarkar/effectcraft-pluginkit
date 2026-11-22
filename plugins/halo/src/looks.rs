pub type Rgb = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Element {
    pub brightness: f32,
    pub size: f32,
    pub shape: f32,
    pub colour: Rgb,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub glow: Element,
    pub rays: Element,
    pub ray_count: u32,
    pub streak: Element,
    pub ring: Element,
    pub fringe: f32,
    pub ghosts: Element,
    pub ghost_count: u32,
    pub blades: u32,
    pub dirt: Element,
    pub hoop: Element,
}

pub const NAMES: [&str; 7] = ["Custom", "Sun", "Anamorphic Blue", "Warm Studio", "Sci-Fi Ring", "Vintage Zoom", "Night Street"];

const fn el(brightness: f32, size: f32, shape: f32, colour: Rgb) -> Element {
    Element { brightness, size, shape, colour }
}

const CUSTOM: Look = Look {
    glow: el(0.6, 60.0, 1.0, [1.0, 0.95, 0.9]),
    rays: el(0.5, 120.0, 12.0, [1.0, 0.95, 0.9]),
    ray_count: 8,
    streak: el(0.3, 300.0, 2.0, [0.9, 0.95, 1.0]),
    ring: el(0.15, 140.0, 3.0, [1.0, 1.0, 1.0]),
    fringe: 1.0,
    ghosts: el(0.25, 40.0, 1.0, [1.0, 1.0, 1.0]),
    ghost_count: 6,
    blades: 6,
    dirt: el(0.1, 260.0, 1.0, [1.0, 0.95, 0.9]),
    hoop: el(0.08, 320.0, 4.0, [0.8, 0.9, 1.0]),
};

const SUN: Look = Look {
    glow: el(0.9, 70.0, 0.9, [1.0, 0.92, 0.78]),
    rays: el(0.8, 220.0, 18.0, [1.0, 0.9, 0.75]),
    ray_count: 14,
    streak: el(0.15, 420.0, 2.5, [1.0, 0.92, 0.8]),
    ring: el(0.12, 180.0, 4.0, [1.0, 0.95, 0.9]),
    fringe: 1.2,
    ghosts: el(0.3, 45.0, 1.0, [1.0, 0.9, 0.75]),
    ghost_count: 7,
    blades: 6,
    dirt: el(0.12, 320.0, 1.0, [1.0, 0.9, 0.75]),
    hoop: el(0.06, 380.0, 6.0, [1.0, 0.85, 0.7]),
};

const ANAMORPHIC_BLUE: Look = Look {
    glow: el(0.7, 45.0, 1.2, [0.7, 0.85, 1.0]),
    rays: el(0.2, 90.0, 30.0, [0.6, 0.8, 1.0]),
    ray_count: 4,
    streak: el(1.1, 900.0, 2.0, [0.25, 0.55, 1.0]),
    ring: el(0.0, 140.0, 3.0, [0.5, 0.7, 1.0]),
    fringe: 0.5,
    ghosts: el(0.3, 34.0, 1.0, [0.45, 0.7, 1.0]),
    ghost_count: 6,
    blades: 0,
    dirt: el(0.05, 200.0, 1.0, [0.5, 0.7, 1.0]),
    hoop: el(0.1, 300.0, 3.0, [0.35, 0.6, 1.0]),
};

const WARM_STUDIO: Look = Look {
    glow: el(0.8, 90.0, 0.8, [1.0, 0.7, 0.4]),
    rays: el(0.3, 160.0, 8.0, [1.0, 0.75, 0.45]),
    ray_count: 6,
    streak: el(0.2, 350.0, 3.0, [1.0, 0.7, 0.45]),
    ring: el(0.18, 160.0, 6.0, [1.0, 0.75, 0.5]),
    fringe: 0.8,
    ghosts: el(0.25, 55.0, 1.0, [1.0, 0.75, 0.5]),
    ghost_count: 5,
    blades: 5,
    dirt: el(0.25, 380.0, 1.0, [1.0, 0.75, 0.5]),
    hoop: el(0.05, 420.0, 8.0, [1.0, 0.7, 0.4]),
};

const SCI_FI_RING: Look = Look {
    glow: el(0.6, 40.0, 1.3, [0.6, 1.0, 1.0]),
    rays: el(0.4, 140.0, 40.0, [0.5, 0.95, 1.0]),
    ray_count: 4,
    streak: el(0.5, 600.0, 1.2, [0.4, 0.9, 1.0]),
    ring: el(0.6, 120.0, 3.5, [0.6, 1.0, 1.0]),
    fringe: 1.2,
    ghosts: el(0.25, 30.0, 1.0, [0.5, 1.0, 0.9]),
    ghost_count: 4,
    blades: 8,
    dirt: el(0.0, 200.0, 1.0, [0.5, 1.0, 1.0]),
    hoop: el(0.35, 260.0, 3.0, [0.5, 0.9, 1.0]),
};

const VINTAGE_ZOOM: Look = Look {
    glow: el(0.7, 80.0, 1.0, [1.0, 0.85, 0.6]),
    rays: el(0.25, 140.0, 10.0, [1.0, 0.85, 0.6]),
    ray_count: 10,
    streak: el(0.1, 300.0, 3.0, [1.0, 0.85, 0.7]),
    ring: el(0.2, 200.0, 8.0, [1.0, 0.9, 0.7]),
    fringe: 1.5,
    ghosts: el(0.4, 50.0, 1.4, [1.0, 0.85, 0.65]),
    ghost_count: 12,
    blades: 7,
    dirt: el(0.2, 300.0, 1.0, [1.0, 0.85, 0.6]),
    hoop: el(0.15, 360.0, 10.0, [1.0, 0.8, 0.55]),
};

const NIGHT_STREET: Look = Look {
    glow: el(1.0, 30.0, 1.1, [1.0, 0.6, 0.3]),
    rays: el(0.6, 90.0, 60.0, [1.0, 0.65, 0.4]),
    ray_count: 16,
    streak: el(0.3, 250.0, 1.5, [1.0, 0.55, 0.45]),
    ring: el(0.08, 70.0, 2.0, [1.0, 0.6, 0.5]),
    fringe: 1.0,
    ghosts: el(0.25, 20.0, 0.8, [1.0, 0.55, 0.6]),
    ghost_count: 5,
    blades: 6,
    dirt: el(0.08, 150.0, 1.0, [1.0, 0.6, 0.4]),
    hoop: el(0.0, 200.0, 3.0, [1.0, 0.6, 0.4]),
};

pub fn look(index: usize) -> Look {
    match index {
        1 => SUN,
        2 => ANAMORPHIC_BLUE,
        3 => WARM_STUDIO,
        4 => SCI_FI_RING,
        5 => VINTAGE_ZOOM,
        6 => NIGHT_STREET,
        _ => CUSTOM,
    }
}
