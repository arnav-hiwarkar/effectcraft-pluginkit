use std::ops::Deref;

use crate::Result;
use crate::abi::Buffer;
use crate::host;
use crate::ui::Ui;
use crate::viewer::Overlay;
use crate::wire::{EffectSpec, HostCall, ImageDesc, Layout, MaskPath, ParamsAt, Pixel, Rect, RenderCx, Scene, Value};

pub trait Effect: Send + Sync + 'static {
    fn spec(&self) -> EffectSpec;

    fn layout(&self, _cx: &Cx) -> Layout {
        Layout::whole_frame()
    }

    fn render(&self, _cx: &Cx, frame: &mut Frame) -> Result<()> {
        frame.map(|pixel| pixel);
        Ok(())
    }

    fn input(&self, _cx: &Cx, out: Rect) -> Rect {
        out
    }

    fn controls(&self, _instance: u64, _param: &str, _value: &mut String, _ui: &mut Ui) {}

    fn pass(&self, _cx: &Cx, pass: &Pass, _out: &mut Frame) -> Result<()> {
        Err(format!(
            "the plan names the kernel `{}` but this effect has no CPU code for it; handle it in Effect::pass or use a host kernel such as GpuPass::blur",
            pass.kernel
        )
        .into())
    }

    fn overlay(&self, _cx: &Cx, _overlay: &mut Overlay) {}

    fn drag(&self, _cx: &Cx, _key: &str, _to: [f64; 2], _value: &mut String) {}
}

pub struct Pass<'a> {
    pub kernel: &'a str,
    pub f: &'a [f32],
    pub u: &'a [u32],
    pub src: Rows<'a>,
    pub aux: Option<Rows<'a>>,
    pub out_size: [usize; 2],
}

#[derive(Clone, Copy)]
pub struct Rows<'a> {
    pub width: usize,
    pub height: usize,
    pub first_row: usize,
    pub pixels: &'a [Pixel],
}

impl<'a> Rows<'a> {
    pub fn row(&self, y: isize) -> Option<&'a [Pixel]> {
        let held = usize::try_from(y).ok()?.checked_sub(self.first_row)?;
        self.pixels.chunks_exact(self.width.max(1)).nth(held)
    }

    pub fn at(&self, x: isize, y: isize) -> Pixel {
        let pixel = usize::try_from(x).ok().and_then(|x| self.row(y)?.get(x));
        pixel.copied().unwrap_or([0.0; 4])
    }
}

pub struct Cx(RenderCx);

impl Deref for Cx {
    type Target = RenderCx;

    fn deref(&self) -> &RenderCx {
        &self.0
    }
}

impl Cx {
    pub fn new(cx: RenderCx) -> Self {
        Cx(cx)
    }

    pub fn f(&self, id: &str) -> f64 {
        match self.params.get(id) {
            Some(Value::Number(n)) => *n,
            Some(Value::Bool(b)) => f64::from(u8::from(*b)),
            _ => 0.0,
        }
    }

    pub fn padded(&self, padding: u32) -> Cx {
        self.padded_sides(padding, padding, padding, padding)
    }

    pub fn padded_sides(&self, left: u32, top: u32, right: u32, bottom: u32) -> Cx {
        let shift = [f64::from(left), f64::from(top)];
        let mut padded = self.0.clone();
        padded.size = [padded.size[0] + left + right, padded.size[1] + top + bottom];
        padded.offset = [padded.offset[0] + shift[0], padded.offset[1] + shift[1]];
        for value in padded.params.values_mut() {
            if let Value::List(point) = value
                && point.len() < 4
            {
                point.iter_mut().zip(shift).for_each(|(c, by)| *c += by);
            }
        }
        Cx(padded)
    }

    pub fn px(&self, id: &str) -> f64 {
        self.f(id) * self.scale
    }

    pub fn bool(&self, id: &str) -> bool {
        match self.params.get(id) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => *n != 0.0,
            _ => false,
        }
    }

    pub fn index(&self, id: &str) -> usize {
        self.f(id).max(0.0) as usize
    }

    pub fn list<const N: usize>(&self, id: &str) -> [f64; N] {
        let mut out = [0.0; N];
        if let Some(Value::List(values)) = self.params.get(id) {
            out.iter_mut().zip(values).for_each(|(slot, v)| *slot = *v);
        }
        out
    }

    pub fn point(&self, id: &str) -> [f64; 2] {
        self.list(id)
    }

    pub fn point3(&self, id: &str) -> [f64; 3] {
        self.list(id)
    }

    pub fn color(&self, id: &str) -> [f32; 4] {
        self.list::<4>(id).map(|c| c as f32)
    }

    pub fn text(&self, id: &str) -> &str {
        match self.params.get(id) {
            Some(Value::Text(text)) => text,
            _ => "",
        }
    }

    pub fn layer(&self, param: &str) -> Option<Image> {
        image(&HostCall::Layer { param: param.into(), time: None })
    }

    pub fn layer_at(&self, param: &str, comp_time: f64) -> Option<Image> {
        image(&HostCall::Layer { param: param.into(), time: Some(comp_time) })
    }

    pub fn source_at(&self, layer_time: f64) -> Option<Image> {
        image(&HostCall::Source { time: layer_time })
    }

    pub fn masks(&self) -> Vec<MaskPath> {
        host::call(&HostCall::Masks, &[]).map(|(masks, _)| masks).unwrap_or_default()
    }

    pub fn recall(&self, name: &str) -> Option<Vec<u8>> {
        host::cache_get(&format!("{}/{name}", self.instance))
    }

    pub fn remember(&self, name: &str, bytes: &[u8]) -> Result<()> {
        host::cache_put(&format!("{}/{name}", self.instance), bytes)
    }

    pub fn at(&self, layer_time: f64) -> Option<Cx> {
        let answer = params_at(layer_time)?;
        let mut cx = self.0.clone();
        cx.comp_time += layer_time - cx.time;
        cx.time = layer_time;
        cx.params = answer.params;
        Some(Cx(cx))
    }

    pub fn param_history(&self) -> Option<u64> {
        params_at(self.time)?.history
    }

    pub fn scene(&self) -> Option<Scene> {
        host::call(&HostCall::Scene, &[]).ok().map(|(scene, _)| scene)
    }
}

fn params_at(time: f64) -> Option<ParamsAt> {
    host::call(&HostCall::ParamsAt { time }, &[]).ok().map(|(answer, _)| answer)
}

fn image(call: &HostCall) -> Option<Image> {
    let (desc, buffer) = host::call::<ImageDesc>(call, &[]).ok()?;
    image_of(desc, buffer)
}

pub(crate) fn image_of(desc: ImageDesc, buffer: crate::abi::Buffer) -> Option<Image> {
    let (width, height) = (desc.width as usize, desc.height as usize);
    (width * height > 0 && buffer.pixels().len() == width * height).then_some(Image { width, height, buffer })
}

pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub origin: [usize; 2],
    pub pixels: &'a mut [Pixel],
    pub input: Rows<'a>,
}

impl<'a> Frame<'a> {
    pub fn whole(width: usize, height: usize, input: &'a [Pixel], pixels: &'a mut [Pixel]) -> Frame<'a> {
        Frame { width, height, origin: [0, 0], pixels, input: Rows { width, height, first_row: 0, pixels: input } }
    }

    pub fn rows_mut(&mut self) -> impl Iterator<Item = (usize, &mut [Pixel])> {
        let first = self.origin[1];
        self.pixels.chunks_exact_mut(self.width.max(1)).enumerate().map(move |(y, row)| (first + y, row))
    }

    pub fn rows(&mut self) -> impl Iterator<Item = (usize, &'a [Pixel], &mut [Pixel])> {
        let ([x0, y0], width, input) = (self.origin, self.width, self.input);
        self.pixels.chunks_exact_mut(width.max(1)).enumerate().map(move |(i, out)| {
            let y = y0 + i;
            let read = input.row(y as isize).and_then(|row| row.get(x0..x0 + width)).unwrap_or(&[]);
            (y, read, out)
        })
    }

    pub fn map(&mut self, f: impl Fn(Pixel) -> Pixel) {
        for (_, read, out) in self.rows() {
            out.iter_mut().zip(read).for_each(|(out, read)| *out = f(*read));
        }
    }

    pub fn at(&self, x: isize, y: isize) -> Pixel {
        self.input.at(x, y)
    }
}

pub struct Image {
    pub width: usize,
    pub height: usize,
    buffer: Buffer,
}

impl Image {
    pub fn pixels(&self) -> &[Pixel] {
        self.buffer.pixels()
    }

    pub fn at(&self, x: isize, y: isize) -> Pixel {
        at(self.pixels(), self.width, self.height, x, y)
    }

    pub fn sample(&self, x: f64, y: f64) -> Pixel {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let mix = |a: Pixel, b: Pixel, t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        let top: Pixel = mix(self.at(x0, y0), self.at(x0 + 1, y0), fx);
        let bottom: Pixel = mix(self.at(x0, y0 + 1), self.at(x0 + 1, y0 + 1), fx);
        mix(top, bottom, fy)
    }
}

fn at(pixels: &[Pixel], width: usize, height: usize, x: isize, y: isize) -> Pixel {
    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
        return [0.0; 4];
    }
    pixels[y as usize * width + x as usize]
}
