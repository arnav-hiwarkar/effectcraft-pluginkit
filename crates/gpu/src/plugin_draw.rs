use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use effectcraft_effects::dynamic::{GpuBlend, GpuCull, GpuDraw, GpuPass, GpuProgram};

use crate::context::{Enc, GpuContext, GpuImage, Kernel, KernelImages, Params};

const MAX_ATTRIBUTES: usize = 8;
const MAX_BUFFER_BYTES: usize = 256 << 20;
const KEPT_BYTES: usize = 512 << 20;
const KEPT_ENTRIES: usize = 256;
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const RESOLVE: &str = "craft_resolve";
const BLIT: &str = "craft_blit";

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct DrawKey {
    pub kernel: String,
    pub layout: Vec<u32>,
    pub instance_layout: Vec<u32>,
    pub depth: bool,
    pub blend: GpuBlend,
    pub cull: GpuCull,
}

#[derive(Clone)]
enum Kept {
    Buffer(wgpu::Buffer),
    Image(GpuImage),
}

struct Upload {
    source: Weak<[u8]>,
    hash: u64,
    bytes: usize,
    used: u64,
    kept: Kept,
}

#[derive(Default)]
struct Uploads {
    entries: HashMap<(String, String, Option<[u32; 2]>), Upload>,
    clock: u64,
}

#[derive(Default)]
pub(crate) struct Store {
    layouts: OnceLock<(wgpu::BindGroupLayout, wgpu::PipelineLayout)>,
    uploads: Mutex<Uploads>,
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

impl Store {
    fn layouts(&self, device: &wgpu::Device) -> &(wgpu::BindGroupLayout, wgpu::PipelineLayout) {
        self.layouts.get_or_init(|| {
            let texture = |binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let uniform = wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            };
            let bind =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("plugin draw"), entries: &[uniform, texture(1), texture(2)] });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("plugin draw"),
                bind_group_layouts: &[Some(&bind)],
                immediate_size: 0,
            });
            (bind, layout)
        })
    }

    pub(crate) fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Ok(mut uploads) = self.uploads.lock() {
            uploads.entries.retain(|(effect, _, _), _| keep(effect));
        }
    }

    fn kept(&self, effect: &str, name: &str, picture: Option<[u32; 2]>, bytes: &Arc<[u8]>, upload: impl FnOnce() -> Kept) -> Option<Kept> {
        let mut uploads = self.uploads.lock().ok()?;
        uploads.clock += 1;
        let clock = uploads.clock;
        let key = (effect.to_string(), name.to_string(), picture);
        let hash = match uploads.entries.get_mut(&key) {
            Some(entry) if std::ptr::addr_eq(entry.source.as_ptr(), Arc::as_ptr(bytes)) => {
                entry.used = clock;
                return Some(entry.kept.clone());
            }
            Some(entry) => {
                let hash = hash_of(bytes);
                if entry.hash == hash && entry.bytes == bytes.len() {
                    entry.source = Arc::downgrade(bytes);
                    entry.used = clock;
                    return Some(entry.kept.clone());
                }
                hash
            }
            None => hash_of(bytes),
        };
        let kept = upload();
        uploads.entries.insert(key, Upload { source: Arc::downgrade(bytes), hash, bytes: bytes.len(), used: clock, kept: kept.clone() });
        while uploads.entries.len() > KEPT_ENTRIES || uploads.entries.values().map(|entry| entry.bytes).sum::<usize>() > KEPT_BYTES {
            let Some(oldest) = uploads.entries.iter().filter(|(_, entry)| entry.used != clock).min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone())
            else {
                break;
            };
            uploads.entries.remove(&oldest);
        }
        Some(kept)
    }
}

fn stride(layout: &[u32]) -> Result<usize, String> {
    if !(1..=MAX_ATTRIBUTES).contains(&layout.len()) || layout.iter().any(|floats| !(1..=4).contains(floats)) {
        return Err(format!("a buffer has 1 to {MAX_ATTRIBUTES} attributes of 1 to 4 floats, not {layout:?}"));
    }
    Ok(layout.iter().sum::<u32>() as usize * 4)
}

pub(crate) fn check_data(program: &GpuProgram, fits: impl Fn(u32, u32) -> bool) -> Result<(), String> {
    if let Some(odd) = program.buffers.iter().find(|buffer| !buffer.bytes.len().is_multiple_of(4)) {
        return Err(format!("buffer {:?} is {} bytes, not a whole number of 4-byte values", odd.name, odd.bytes.len()));
    }
    let bytes: usize = program.buffers.iter().map(|buffer| buffer.bytes.len()).sum();
    if bytes > MAX_BUFFER_BYTES {
        return Err(format!("the buffers hold {bytes} bytes; the limit is {MAX_BUFFER_BYTES}"));
    }
    for texture in &program.textures {
        let [w, h] = texture.size;
        let area = u64::from(w) * u64::from(h);
        if w == 0 || h == 0 || !fits(w, h) || texture.bytes.len() as u64 != area * 16 {
            return Err(format!(
                "texture {:?} is {w}x{h} with {} bytes; it needs 16 bytes for each pixel and a size the GPU holds",
                texture.name,
                texture.bytes.len()
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub elements: u32,
    pub instances: u32,
}

pub(crate) fn check_draw(n: usize, draw: &GpuDraw, program: &GpuProgram, scan_indices: bool) -> Result<Counts, String> {
    let buffer = |index: usize, what: &str| {
        program.buffers.get(index).map(|buffer| &*buffer.bytes).ok_or_else(|| format!("pass {n} names {what} buffer {index}, which the plan does not declare"))
    };
    let whole = |bytes: &[u8], stride: usize, what: &str| match bytes.len() / stride {
        count if count > 0 && bytes.len().is_multiple_of(stride) => u32::try_from(count).map_err(|_| format!("pass {n}: too many {what}")),
        _ => Err(format!("pass {n}: the {what} buffer is {} bytes, not one or more whole {stride}-byte {what}", bytes.len())),
    };
    let vertices = whole(buffer(draw.vertices, "vertex")?, stride(&draw.layout)?, "vertices")?;
    let instances = match draw.instances {
        Some(index) => whole(buffer(index, "instance")?, stride(&draw.instance_layout)?, "instances")?,
        None if draw.instance_layout.is_empty() => 1,
        None => return Err(format!("pass {n} has an instance layout and no instance buffer")),
    };
    let available = match draw.indices {
        Some(index) => {
            let bytes = buffer(index, "index")?;
            let count = whole(bytes, 4, "indices")?;
            let used = draw.count.unwrap_or(count).min(count) as usize;
            let outside = |index: &[u8; 4]| u32::from_le_bytes(*index) >= vertices;
            if scan_indices && bytes.as_chunks::<4>().0.iter().take(used).any(outside) {
                return Err(format!("pass {n} has an index past its {vertices} vertices"));
            }
            count
        }
        None => vertices,
    };
    let elements = draw.count.unwrap_or(available);
    if elements > available {
        return Err(format!("pass {n} draws {elements} elements of {available}"));
    }
    Ok(Counts { elements, instances })
}

fn attributes(layout: &[u32], first: u32) -> Vec<wgpu::VertexAttribute> {
    let mut offset = 0;
    let attribute = |(i, floats): (usize, &u32)| {
        let format = match floats {
            1 => wgpu::VertexFormat::Float32,
            2 => wgpu::VertexFormat::Float32x2,
            3 => wgpu::VertexFormat::Float32x3,
            _ => wgpu::VertexFormat::Float32x4,
        };
        let at = offset;
        offset += u64::from(*floats) * 4;
        wgpu::VertexAttribute { format, offset: at, shader_location: first + i as u32 }
    };
    layout.iter().enumerate().map(attribute).collect()
}

pub(crate) fn pipeline(g: &GpuContext, module: &wgpu::ShaderModule, key: &DrawKey) -> wgpu::RenderPipeline {
    let (_, layout) = g.plugin_draw.layouts(&g.device);
    let vertex = attributes(&key.layout, 0);
    let instance = attributes(&key.instance_layout, key.layout.len() as u32);
    let size = |layout: &[u32]| u64::from(layout.iter().sum::<u32>()) * 4;
    let mut buffers = Vec::new();
    if !vertex.is_empty() {
        buffers.push(Some(wgpu::VertexBufferLayout { array_stride: size(&key.layout), step_mode: wgpu::VertexStepMode::Vertex, attributes: &vertex }));
    }
    if !instance.is_empty() {
        buffers.push(Some(wgpu::VertexBufferLayout {
            array_stride: size(&key.instance_layout),
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance,
        }));
    }
    let blend = match key.blend {
        GpuBlend::Replace => None,
        GpuBlend::Over => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        GpuBlend::Add => {
            let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
            Some(wgpu::BlendState { color: add, alpha: add })
        }
    };
    let cull_mode = match key.cull {
        GpuCull::None => None,
        GpuCull::Back => Some(wgpu::Face::Back),
        GpuCull::Front => Some(wgpu::Face::Front),
    };
    g.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&key.kernel),
        layout: Some(layout),
        vertex: wgpu::VertexState { module, entry_point: Some(&format!("vs_{}", key.kernel)), compilation_options: Default::default(), buffers: &buffers },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode, ..Default::default() },
        depth_stencil: key.depth.then(|| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(&format!("fs_{}", key.kernel)),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: COLOUR, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

struct Canvas {
    target: usize,
    colour: wgpu::Texture,
}

pub(crate) struct Stage<'a> {
    pub effect: &'a str,
    pub revision: u64,
    pub wgsl: &'a str,
    pub program: &'a GpuProgram,
    depth: Option<wgpu::Texture>,
    canvas: Option<Canvas>,
}

pub(crate) struct Target<'a> {
    pub index: usize,
    pub size: [u32; 2],
    pub below: Option<&'a GpuImage>,
    pub stays_open: bool,
}

impl<'a> Stage<'a> {
    pub fn new(effect: &'a str, revision: u64, wgsl: &'a str, program: &'a GpuProgram) -> Self {
        Stage { effect, revision, wgsl, program, depth: None, canvas: None }
    }

    pub fn texture(&self, g: &GpuContext, index: usize) -> Option<GpuImage> {
        let picture = self.program.textures.get(index)?;
        let [w, h] = picture.size;
        let upload = || {
            let image = g.image(w, h);
            g.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &image.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &picture.bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            Kept::Image(image)
        };
        match g.plugin_draw.kept(self.effect, &picture.name, Some(picture.size), &picture.bytes, upload)? {
            Kept::Image(image) => Some(image),
            Kept::Buffer(_) => None,
        }
    }

    fn buffer(&self, g: &GpuContext, index: usize) -> Option<wgpu::Buffer> {
        let data = self.program.buffers.get(index)?;
        let upload = || Kept::Buffer(g.buffer_init(Some(&data.name), &data.bytes, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX));
        match g.plugin_draw.kept(self.effect, &data.name, None, &data.bytes, upload)? {
            Kept::Buffer(buffer) => Some(buffer),
            Kept::Image(_) => None,
        }
    }

    fn attachment(g: &GpuContext, [w, h]: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
        g.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("plugin draw target"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn bind(g: &GpuContext, params: &Params, src: &wgpu::Texture, aux: Option<&wgpu::Texture>) -> wgpu::BindGroup {
        let (layout, _) = g.plugin_draw.layouts(&g.device);
        let uniform = g.buffer_init(None, &params.bytes(), wgpu::BufferUsages::UNIFORM);
        let src = src.create_view(&Default::default());
        let aux = aux.map(|aux| aux.create_view(&Default::default()));
        g.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(aux.as_ref().unwrap_or(&g.dummy_tex)) },
            ],
        })
    }

    pub fn draw(
        &mut self,
        e: &mut Enc,
        pass: &GpuPass,
        params: &Params,
        src: &GpuImage,
        aux: Option<&GpuImage>,
        target: Target,
    ) -> Result<Option<GpuImage>, String> {
        let g = e.g;
        let draw = pass.draw.as_ref().ok_or("not a draw pass")?;
        let counts = check_draw(0, draw, self.program, false)?;
        let key = DrawKey {
            kernel: pass.kernel.clone(),
            layout: draw.layout.clone(),
            instance_layout: draw.instance_layout.clone(),
            depth: draw.depth,
            blend: draw.blend,
            cull: draw.cull,
        };
        let pipeline = g.plugin_draw_pipeline(self.effect, self.revision, self.wgsl, &key)?;
        let gone = || "a GPU buffer could not be kept".to_string();
        let vertices = self.buffer(g, draw.vertices).ok_or_else(gone)?;
        let indices = draw.indices.map(|index| self.buffer(g, index).ok_or_else(gone)).transpose()?;
        let instances = draw.instances.map(|index| self.buffer(g, index).ok_or_else(gone)).transpose()?;
        let open = self.canvas.take().filter(|canvas| canvas.target == target.index && !draw.clear);
        let underlay = match target.below {
            Some(below) if open.is_none() && !draw.clear => {
                let blit = g.plugin_draw_pipeline(self.effect, self.revision, self.wgsl, &DrawKey { kernel: BLIT.into(), ..Default::default() })?;
                Some((blit, Self::bind(g, &Params::default(), &below.texture, None)))
            }
            _ => None,
        };
        let load = if open.is_some() || underlay.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) };
        let colour = open.map_or_else(|| Self::attachment(g, target.size, COLOUR), |canvas| canvas.colour);
        let fresh_depth = draw.depth && self.depth.is_none();
        if fresh_depth {
            self.depth = Some(Self::attachment(g, target.size, DEPTH));
        }
        let bound = Self::bind(g, params, &src.texture, aux.map(|aux| &aux.texture));
        let colour_view = colour.create_view(&Default::default());
        let depth_view = self.depth.as_ref().filter(|_| draw.depth).map(|depth| depth.create_view(&Default::default()));
        let begin = |enc: &'_ mut wgpu::CommandEncoder, load, depth: Option<&wgpu::TextureView>, clear_depth: bool| {
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plugin draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &colour_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: if clear_depth { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime()
        };
        let images: Vec<&GpuImage> = [Some(src), aux, target.below].into_iter().flatten().collect();
        let enc = e.record(&images);
        if let Some((blit, bind)) = &underlay {
            let mut under = begin(enc, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), None, false);
            under.set_pipeline(blit);
            under.set_bind_group(0, bind, &[]);
            under.draw(0..3, 0..1);
        }
        {
            let mut render = begin(enc, load, depth_view.as_ref(), fresh_depth);
            render.set_pipeline(&pipeline);
            render.set_bind_group(0, &bound, &[]);
            render.set_vertex_buffer(0, vertices.slice(..));
            if let Some(instances) = &instances {
                render.set_vertex_buffer(1, instances.slice(..));
            }
            match &indices {
                Some(indices) => {
                    render.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                    render.draw_indexed(0..counts.elements, 0, 0..counts.instances);
                }
                None => render.draw(0..counts.elements, 0..counts.instances),
            }
        }
        if target.stays_open {
            self.canvas = Some(Canvas { target: target.index, colour });
            return Ok(None);
        }
        let resolve = g.plugin_pipelines(self.effect, self.revision, self.wgsl, &[RESOLVE])?.pop().ok_or("no resolve kernel")?;
        let [w, h] = target.size;
        let out = e.scratch(w, h);
        let drawn = GpuImage::new(colour, w, h);
        let bound = KernelImages { src: &drawn, aux: None, out: &out };
        e.dispatch(Kernel::Plugin { pipeline: &resolve, label: RESOLVE }, &Params::default(), bound, None, (w.div_ceil(16), h.div_ceil(16)));
        Ok(Some(out))
    }
}
