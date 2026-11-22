use crate::noise;
use crate::random::{Stream, mix};
use crate::settings::{Emitter, Sim, Vec3};

pub const SUBSTEPS: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
    pub life: f32,
    pub seed: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub frame: u32,
    pub carry: f64,
    pub particles: Vec<Particle>,
}

struct MaskPath {
    points: Vec<[f32; 2]>,
    lengths: Vec<f32>,
}

pub struct Simulator<'a> {
    base: &'a Sim,
    masks: Vec<MaskPath>,
    total_length: f32,
    dt: f32,
}

struct Emission {
    centre: Vec3,
    motion: Vec3,
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Vec3, s: f32) -> Vec3 {
    a.map(|v| v * s)
}

fn basis(direction: Vec3) -> (Vec3, Vec3) {
    let helper = if direction[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let cross = |a: Vec3, b: Vec3| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let u = cross(direction, helper);
    let len = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt().max(1e-12);
    let u = scale(u, 1.0 / len);
    (u, cross(direction, u))
}

impl<'a> Simulator<'a> {
    pub fn new(base: &'a Sim) -> Simulator<'a> {
        let masks: Vec<MaskPath> = match &base.emitter {
            Emitter::Masks(paths) => paths
                .iter()
                .filter(|p| !p.is_empty())
                .map(|points| {
                    let mut total = 0.0;
                    let lengths = std::iter::once(0.0)
                        .chain(points.windows(2).map(|w| {
                            total += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
                            total
                        }))
                        .collect();
                    MaskPath { points: points.clone(), lengths }
                })
                .collect(),
            _ => Vec::new(),
        };
        let total_length = masks.iter().map(|m| m.lengths.last().copied().unwrap_or(0.0)).sum();
        let dt = 1.0 / (base.frame_rate * SUBSTEPS as f32);
        Simulator { base, masks, total_length, dt }
    }

    fn mask_point(&self, rng: &mut Stream) -> Option<[f32; 2]> {
        if self.masks.is_empty() {
            return None;
        }
        if self.total_length <= 0.0 {
            let mask = &self.masks[(rng.next() * self.masks.len() as f32) as usize % self.masks.len()];
            return mask.points.first().copied();
        }
        let mut at = rng.next() * self.total_length;
        for mask in &self.masks {
            let length = mask.lengths.last().copied().unwrap_or(0.0);
            if at <= length {
                let i = mask.lengths.partition_point(|l| *l < at).clamp(1, mask.points.len().max(2) - 1);
                let (a, b) = (mask.points[i - 1], mask.points[i.min(mask.points.len() - 1)]);
                let span = (mask.lengths[i] - mask.lengths[i - 1]).max(1e-9);
                let t = ((at - mask.lengths[i - 1]) / span).clamp(0.0, 1.0);
                return Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
            at -= length;
        }
        self.masks.last().and_then(|m| m.points.last().copied())
    }

    fn spawn(&self, sim: &Sim, emission: &Emission, step: u64, index: u32) -> Option<Particle> {
        let seed = mix(mix(self.base.seed, step as u32), mix((step >> 32) as u32, index));
        let mut rng = Stream::new(seed);
        let centre = emission.centre;
        let half = scale(sim.extent, 0.5);
        let position = match self.base.emitter {
            Emitter::Point => centre,
            Emitter::Box => [0, 1, 2].map(|a| centre[a] + rng.signed() * half[a]),
            Emitter::Sphere => {
                let p = rng.in_sphere();
                [0, 1, 2].map(|a| centre[a] + p[a] * half[a])
            }
            Emitter::Masks(_) => {
                let p = self.mask_point(&mut rng)?;
                [p[0], p[1], centre[2]]
            }
        };
        let (u, v) = basis(sim.direction);
        let cone = sim.spread * std::f32::consts::PI;
        let cos_theta = 1.0 - rng.next() * (1.0 - cone.cos());
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let phi = rng.next() * std::f32::consts::TAU;
        let heading = [0, 1, 2].map(|a| sim.direction[a] * cos_theta + (u[a] * phi.cos() + v[a] * phi.sin()) * sin_theta);
        let speed = sim.speed * (1.0 + sim.speed_random * rng.signed());
        let velocity = add(scale(heading, speed), scale(emission.motion, sim.inherit));
        let life = sim.life * (1.0 + sim.life_random * rng.signed());
        Some(Particle { position, velocity, age: 0.0, life: life.max(self.dt), seed: mix(seed, 0x5eed) })
    }

    fn born(&self, sim: &Sim, step: u64, carry: &mut f64) -> u64 {
        if self.base.burst {
            return if step == 0 { sim.rate as u64 } else { 0 };
        }
        let due = *carry + f64::from(sim.rate) / (f64::from(self.base.frame_rate) * f64::from(SUBSTEPS));
        let born = due.floor();
        *carry = due - born;
        born as u64
    }

    fn substep(&self, sim: &Sim, emission: &Emission, state: &mut State, step: u64) {
        let born = self.born(sim, step, &mut state.carry);
        let particles = &mut state.particles;
        let room = (self.base.max_particles as usize).saturating_sub(particles.len());
        particles.extend((0..born.min(room as u64) as u32).filter_map(|i| self.spawn(sim, emission, step, i)));
        let dt = self.dt;
        let drag = 1.0 - (-sim.drag * dt).exp();
        let time = step as f32 * dt;
        let field_scale = 1.0 / sim.turbulence_scale;
        let evolution = time * sim.turbulence_evolution;
        let turbulence = sim.turbulence;
        let noise_seed = mix(self.base.seed, 0x7a5b);
        particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life {
                return false;
            }
            p.velocity[1] += sim.gravity * dt;
            for a in 0..3 {
                p.velocity[a] += (sim.wind[a] - p.velocity[a]) * drag;
            }
            let flow = if turbulence > 0.0 {
                let q = [p.position[0] * field_scale, p.position[1] * field_scale, p.position[2] * field_scale + evolution];
                scale(noise::curl(q, noise_seed), turbulence)
            } else {
                [0.0; 3]
            };
            for ((position, velocity), flow) in p.position.iter_mut().zip(p.velocity).zip(flow) {
                *position += (velocity + flow) * dt;
            }
            p.position.iter().all(|v| v.is_finite())
        });
    }

    pub fn step_frame(&self, state: &mut State, now: &Sim, after: &Sim) {
        let next = state.frame.wrapping_add(1);
        let motion = scale([0, 1, 2].map(|a| after.position[a] - now.position[a]), self.base.frame_rate);
        for sub in 0..SUBSTEPS {
            let along = sub as f32 / SUBSTEPS as f32;
            let centre = [0, 1, 2].map(|a| now.position[a] + (after.position[a] - now.position[a]) * along);
            self.substep(now, &Emission { centre, motion }, state, u64::from(next) * u64::from(SUBSTEPS) + u64::from(sub));
        }
        state.frame = next;
    }

    pub fn start(&self, now: &Sim, after: &Sim) -> State {
        let mut state = State { frame: u32::MAX, carry: 0.0, particles: Vec::new() };
        self.step_frame(&mut state, now, after);
        state
    }
}

impl State {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.particles.len() * 36);
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.extend_from_slice(&(self.particles.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.carry.to_le_bytes());
        for p in &self.particles {
            for v in p.position.iter().chain(&p.velocity).chain([&p.age, &p.life]) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&p.seed.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<State> {
        let word = |i: usize| bytes.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let frame = word(0)?;
        let count = word(1)? as usize;
        if bytes.len() != 16 + count * 36 {
            return None;
        }
        let carry = f64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
        let particles = (0..count)
            .map(|n| {
                let base = 4 + n * 9;
                let f = |k: usize| f32::from_bits(word(base + k).unwrap_or(0));
                Particle { position: [f(0), f(1), f(2)], velocity: [f(3), f(4), f(5)], age: f(6), life: f(7), seed: word(base + 8).unwrap_or(0) }
            })
            .collect();
        Some(State { frame, carry, particles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::direction;

    pub fn fountain() -> Sim {
        Sim {
            emitter: Emitter::Point,
            position: [100.0, 200.0, 0.0],
            extent: [50.0; 3],
            rate: 300.0,
            burst: false,
            direction: direction(0.0, 0.0),
            spread: 0.1,
            speed: 400.0,
            speed_random: 0.2,
            inherit: 0.0,
            seed: 3,
            pre_run_frames: 0,
            max_particles: 100_000,
            gravity: 300.0,
            wind: [0.0; 3],
            drag: 0.0,
            turbulence: 0.0,
            turbulence_scale: 100.0,
            turbulence_evolution: 0.0,
            life: 2.0,
            life_random: 0.0,
            frame_rate: 30.0,
        }
    }

    fn run(sim: &Sim, frames: u32) -> State {
        let s = Simulator::new(sim);
        let mut state = s.start(sim, sim);
        (0..frames).for_each(|_| s.step_frame(&mut state, sim, sim));
        state
    }

    #[test]
    fn the_rate_sets_how_many_particles_are_born_per_second() {
        let state = run(&fountain(), 29);
        assert_eq!(state.frame, 29);
        assert_eq!(state.particles.len(), 300);
    }

    #[test]
    fn particles_die_at_the_end_of_their_life() {
        let state = run(&fountain(), 89);
        assert_eq!(state.particles.len(), 600);
        assert!(state.particles.iter().all(|p| p.age < 2.0));
    }

    #[test]
    fn gravity_bends_a_fountain_back_down() {
        let state = run(&fountain(), 59);
        let oldest = state.particles.iter().max_by(|a, b| a.age.total_cmp(&b.age)).expect("particles");
        assert!(oldest.velocity[1] > 150.0, "falling at {}", oldest.velocity[1]);
        let youngest = state.particles.iter().min_by(|a, b| a.age.total_cmp(&b.age)).expect("particles");
        assert!(youngest.velocity[1] < -250.0, "rising at {}", youngest.velocity[1]);
    }

    #[test]
    fn a_burst_emits_everything_in_the_first_step() {
        assert_eq!(run(&Sim { burst: true, ..fountain() }, 0).particles.len(), 300);
        assert_eq!(run(&Sim { burst: true, ..fountain() }, 20).particles.len(), 300);
    }

    #[test]
    fn the_particle_cap_holds() {
        let state = run(&Sim { max_particles: 50, ..fountain() }, 59);
        assert_eq!(state.particles.len(), 50);
    }

    #[test]
    fn drag_settles_particles_onto_the_wind() {
        let state = run(&Sim { drag: 20.0, gravity: 0.0, wind: [120.0, 0.0, 0.0], ..fountain() }, 30);
        let p = state.particles.iter().max_by(|a, b| a.age.total_cmp(&b.age)).expect("particles");
        assert!((p.velocity[0] - 120.0).abs() < 0.5 && p.velocity[1].abs() < 0.5, "{:?}", p.velocity);
    }

    #[test]
    fn mask_emitters_spawn_on_the_path() {
        let sim = Sim { emitter: Emitter::Masks(vec![vec![[10.0, 50.0], [210.0, 50.0]]]), speed: 0.0, gravity: 0.0, ..fountain() };
        let state = run(&sim, 0);
        assert_eq!(state.particles.len(), 10);
        assert!(state.particles.iter().all(|p| (p.position[1] - 50.0).abs() < 1e-3 && (10.0..=210.0).contains(&p.position[0])));
    }

    #[test]
    fn a_state_survives_its_byte_form() {
        let state = run(&Sim { turbulence: 200.0, ..fountain() }, 12);
        assert_eq!(State::from_bytes(&state.to_bytes()), Some(state));
        assert_eq!(State::from_bytes(&[1, 2, 3]), None);
    }
}
