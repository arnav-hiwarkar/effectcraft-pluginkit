        let gain = (cx.f("amount") / 100.0) as f32;
        Layout::pointwise().gpu(GpuPlan::new().pass(GpuPass::new("{{snake}}_apply", GpuImage::Source, GpuImage::Output).floats(&[gain])))
