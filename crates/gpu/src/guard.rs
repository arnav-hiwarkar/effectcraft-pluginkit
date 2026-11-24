use std::collections::{BTreeSet, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use naga::{
    AddressSpace, AtomicFunction, BinaryOperator, Block, Expression, Function, GlobalVariable, Handle, Literal, MemoryDecorations, Module, Range,
    ResourceBinding, Scalar, ShaderStage, Span, Statement, StorageAccess, Type, TypeInner,
};

pub(crate) const LOOP_LIMIT: u32 = 1 << 20;
pub(crate) const GROUP: u32 = 1;

const IDLE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

static STOPPED: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());

#[derive(Default)]
pub(crate) struct Layouts(OnceLock<(wgpu::BindGroupLayout, wgpu::PipelineLayout)>);

impl Layouts {
    pub(crate) fn get(&self, device: &wgpu::Device, kernels: &wgpu::BindGroupLayout) -> &(wgpu::BindGroupLayout, wgpu::PipelineLayout) {
        self.0.get_or_init(|| {
            let flag = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("plugin loop guard"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let compute = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("plugin kernels"),
                bind_group_layouts: &[Some(kernels), Some(&flag)],
                immediate_size: 0,
            });
            (flag, compute)
        })
    }
}

pub(crate) struct ShaderGuard {
    digest: u64,
    flag: wgpu::Buffer,
    staging: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
    state: Arc<AtomicU8>,
}

pub(crate) fn digest(wgsl: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    wgsl.hash(&mut hasher);
    hasher.finish()
}

fn stopped() -> std::sync::MutexGuard<'static, BTreeSet<u64>> {
    STOPPED.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ShaderGuard {
    pub(crate) fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, digest: u64) -> Self {
        let buffer = |usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some("plugin loop guard"), size: 4, usage, mapped_at_creation: false });
        let flag = buffer(wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let staging = buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plugin loop guard"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: flag.as_entire_binding() }],
        });
        ShaderGuard { digest, flag, staging, bind, state: Arc::new(AtomicU8::new(IDLE)) }
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        if self.state.compare_exchange(MAPPED, MAPPING, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            let raised = self.staging.slice(..).get_mapped_range().is_ok_and(|view| view.first().is_some_and(|byte| *byte != 0));
            self.staging.unmap();
            self.state.store(IDLE, Ordering::Release);
            if raised {
                stopped().insert(self.digest);
            }
        }
        match stopped().contains(&self.digest) {
            true => Err(format!(
                "a loop in this shader ran past {LOOP_LIMIT} iterations in one invocation, so its GPU kernels are turned off and the effect renders on the CPU until the WGSL changes; bound every loop"
            )),
            false => Ok(()),
        }
    }

    pub(crate) fn copy(&self, encoder: &mut wgpu::CommandEncoder) -> bool {
        let claimed = self.state.compare_exchange(IDLE, COPIED, Ordering::AcqRel, Ordering::Acquire).is_ok();
        if claimed {
            encoder.copy_buffer_to_buffer(&self.flag, 0, &self.staging, 0, 4);
        }
        claimed
    }

    pub(crate) fn submitted(&self) {
        self.state.store(MAPPING, Ordering::Release);
        let state = self.state.clone();
        self.staging.slice(..).map_async(wgpu::MapMode::Read, move |mapped| state.store(if mapped.is_ok() { MAPPED } else { IDLE }, Ordering::Release));
    }

    pub(crate) fn dropped(&self) {
        let _ = self.state.compare_exchange(COPIED, IDLE, Ordering::AcqRel, Ordering::Acquire);
    }
}

struct Guard {
    count: Handle<GlobalVariable>,
    flag: Handle<GlobalVariable>,
}

pub(crate) fn cap_loops(source: &str) -> Result<Module, naga::front::wgsl::ParseError> {
    let mut module = naga::front::wgsl::parse_str(source)?;
    let mut global = |name: &str, space, binding, inner| {
        let ty = module.types.insert(Type { name: None, inner }, Span::UNDEFINED);
        let variable = GlobalVariable { name: Some(name.into()), space, binding, ty, init: None, memory_decorations: MemoryDecorations::empty() };
        module.global_variables.append(variable, Span::UNDEFINED)
    };
    let count = global("craft_loop_count", AddressSpace::Private, None, TypeInner::Scalar(Scalar::U32));
    let storage = AddressSpace::Storage { access: StorageAccess::LOAD | StorageAccess::STORE };
    let flag = global("craft_loop_flag", storage, Some(ResourceBinding { group: GROUP, binding: 0 }), TypeInner::Atomic(Scalar::U32));
    let guard = Guard { count, flag };
    let mut vertex = HashSet::new();
    for entry in module.entry_points.iter().filter(|entry| entry.stage == ShaderStage::Vertex) {
        called(&module, &entry.function.body, &mut vertex);
    }
    for (handle, function) in module.functions.iter_mut() {
        guard.function(function, !vertex.contains(&handle));
    }
    for entry in &mut module.entry_points {
        guard.function(&mut entry.function, entry.stage != ShaderStage::Vertex);
    }
    Ok(module)
}

fn called(module: &Module, block: &Block, found: &mut HashSet<Handle<Function>>) {
    for statement in block.iter() {
        match statement {
            Statement::Call { function, .. } => {
                if found.insert(*function) {
                    called(module, &module.functions[*function].body, found);
                }
            }
            Statement::Block(inner) => called(module, inner, found),
            Statement::If { accept, reject, .. } => {
                called(module, accept, found);
                called(module, reject, found);
            }
            Statement::Switch { cases, .. } => cases.iter().for_each(|case| called(module, &case.body, found)),
            Statement::Loop { body, continuing, .. } => {
                called(module, body, found);
                called(module, continuing, found);
            }
            _ => {}
        }
    }
}

impl Guard {
    fn function(&self, function: &mut Function, flags: bool) {
        let mut body = std::mem::take(&mut function.body);
        self.block(&mut body, function, flags);
        function.body = body;
    }

    fn block(&self, block: &mut Block, function: &mut Function, flags: bool) {
        for (statement, _) in block.span_iter_mut() {
            match statement {
                Statement::Block(inner) => self.block(inner, function, flags),
                Statement::If { accept, reject, .. } => {
                    self.block(accept, function, flags);
                    self.block(reject, function, flags);
                }
                Statement::Switch { cases, .. } => cases.iter_mut().for_each(|case| self.block(&mut case.body, function, flags)),
                Statement::Loop { body, continuing, .. } => {
                    self.block(body, function, flags);
                    self.block(continuing, function, flags);
                    body.splice(0..0, self.check(function, flags));
                }
                _ => {}
            }
        }
    }

    fn check(&self, function: &mut Function, flags: bool) -> Block {
        let mut add = |expression| function.expressions.append(expression, Span::UNDEFINED);
        let pointer = add(Expression::GlobalVariable(self.count));
        let limit = add(Expression::Literal(Literal::U32(LOOP_LIMIT)));
        let one = add(Expression::Literal(Literal::U32(1)));
        let flag = add(Expression::GlobalVariable(self.flag));
        let count = add(Expression::Load { pointer });
        let spent = add(Expression::Binary { op: BinaryOperator::GreaterEqual, left: count, right: limit });
        let next = add(Expression::Binary { op: BinaryOperator::Add, left: count, right: one });
        let raise = flags.then_some(Statement::Atomic { pointer: flag, fun: AtomicFunction::Max, value: one, result: None });
        Block::from_vec(vec![
            Statement::Emit(Range::new_from_bounds(count, next)),
            Statement::If { condition: spent, accept: Block::from_vec(raise.into_iter().chain([Statement::Break]).collect()), reject: Block::new() },
            Statement::Store { pointer, value: next },
        ])
    }
}
