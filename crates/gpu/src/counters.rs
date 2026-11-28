use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

macro_rules! counters {
    ($($(#[$doc:meta])* $field:ident),* $(,)?) => {
        #[derive(Default)]
        pub(crate) struct Counters {
            $($field: AtomicU64,)*
        }

        /// A snapshot of [`GpuContext::counters`](crate::GpuContext::counters).
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct GpuCounters {
            $($(#[$doc])* pub $field: u64,)*
        }

        impl Counters {
            pub(crate) fn snapshot(&self) -> GpuCounters {
                GpuCounters { $($field: self.$field.load(Relaxed),)* }
            }

            pub(crate) fn reset(&self) {
                $(self.$field.store(0, Relaxed);)*
            }
        }

        impl GpuCounters {
            /// Every counter by name, in declaration order.
            pub fn fields(&self) -> Vec<(&'static str, u64)> {
                vec![$((stringify!($field), self.$field),)*]
            }
        }
    };
}

counters! {
    /// Images uploaded into textures (a cached layer counts once).
    uploads,
    /// Bytes of those uploads.
    upload_bytes,
    /// Storage, vertex and index buffers created with contents from the CPU.
    buffer_uploads,
    /// Bytes written into those buffers.
    buffer_upload_bytes,
    /// Textures read back to the CPU.
    readbacks,
    /// Bytes of those readbacks.
    readback_bytes,
    /// Plugin loop-guard flags copied back to the CPU (4 bytes each).
    loop_flag_reads,
    /// Compute dispatches recorded.
    dispatches,
    /// Draw calls recorded.
    draws,
    /// Compute pipelines built.
    compute_pipelines,
    /// Render pipelines built.
    render_pipelines,
    /// WGSL shader modules created.
    shader_modules,
    /// Uniform buffers allocated.
    uniform_buffers,
    /// Bytes of those uniform buffers.
    uniform_bytes,
    /// Passes of plugin GPU plans recorded (compute and draw).
    plugin_passes,
    /// Layers a plugin pass read that were fitted through a copy of their pixels.
    plugin_layers_copied,
}

impl Counters {
    fn add(counter: &AtomicU64, n: usize) {
        counter.fetch_add(n as u64, Relaxed);
    }

    pub(crate) fn upload(&self, bytes: usize) {
        Self::add(&self.uploads, 1);
        Self::add(&self.upload_bytes, bytes);
    }

    pub(crate) fn readback(&self, bytes: usize) {
        Self::add(&self.readbacks, 1);
        Self::add(&self.readback_bytes, bytes);
    }

    pub(crate) fn buffer(&self, usage: wgpu::BufferUsages, bytes: usize) {
        if usage.contains(wgpu::BufferUsages::UNIFORM) {
            Self::add(&self.uniform_buffers, 1);
            Self::add(&self.uniform_bytes, bytes);
        } else {
            Self::add(&self.buffer_uploads, 1);
            Self::add(&self.buffer_upload_bytes, bytes);
        }
    }

    pub(crate) fn loop_flag_read(&self) {
        Self::add(&self.loop_flag_reads, 1);
    }

    pub(crate) fn dispatch(&self) {
        Self::add(&self.dispatches, 1);
    }

    pub(crate) fn draw(&self) {
        Self::add(&self.draws, 1);
    }

    pub(crate) fn compute_pipeline(&self) {
        Self::add(&self.compute_pipelines, 1);
    }

    pub(crate) fn render_pipeline(&self) {
        Self::add(&self.render_pipelines, 1);
    }

    pub(crate) fn shader_module(&self) {
        Self::add(&self.shader_modules, 1);
    }

    pub(crate) fn plugin_pass(&self) {
        Self::add(&self.plugin_passes, 1);
    }

    pub(crate) fn plugin_layer_copied(&self) {
        Self::add(&self.plugin_layers_copied, 1);
    }
}
