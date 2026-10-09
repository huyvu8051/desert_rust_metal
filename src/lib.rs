//! The Desert City map drawn straight from glTF into Metal buffers, with no
//! Bevy and no ECS — the renderer's own pre-generic data path.
//!
//! It shares the scene shader and the glTF loader with `metal_renderer`, so the
//! only thing that differs from `desert_bevy_baseline` is where the draw list
//! comes from: a `Vec<GltfMesh>` built once at load, against a snapshot
//! extracted from the world every frame.

#![cfg(target_os = "ios")]

mod stats;

use core_graphics::geometry::{CGRect, CGSize};
use metal::foreign_types::ForeignType;
use metal::{
    CAMetalLayer, CompileOptions, Device, MTLClearColor, MTLIndexType, MTLLoadAction,
    MTLPixelFormat, MTLPrimitiveType, MTLResourceOptions, MTLStoreAction, MetalLayer,
    RenderPassDescriptor, RenderPipelineDescriptor, RenderPipelineState,
};
use metal_renderer::gltf_scene::{GltfMesh, load_gltf_scene};
use metal_renderer::metal_math::{look_at, perspective};
use metal_renderer::native_metal_renderer::{SPONZA_SHADER_SOURCE, ShaderFeatures, SponzaUniforms};
use objc::runtime::Object;
use objc::{msg_send, sel, sel_impl};

const MAP_PATH: &str = "assets/maps/desert_city/scene.gltf";

/// Shared with desert_rust_bevy, which orients its DirectionalLight to match.
/// The two rigs have to light the scene identically or their frame costs are
/// not comparable.
pub const SUN_DIRECTION: [f32; 3] = [0.47, -0.78, 0.41];

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value != "0")
}

fn bundle_resource_path() -> Option<String> {
    unsafe {
        let bundle: *mut Object = msg_send![objc::class!(NSBundle), mainBundle];
        let path: *mut Object = msg_send![bundle, resourcePath];
        if path.is_null() {
            return None;
        }
        let utf8: *const std::os::raw::c_char = msg_send![path, UTF8String];
        Some(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub struct RustMetal {
    device: Device,
    queue: metal::CommandQueue,
    layer: MetalLayer,
    pipeline: RenderPipelineState,
    blend_pipeline: RenderPipelineState,
    depth_state: metal::DepthStencilState,
    blend_depth_state: metal::DepthStencilState,
    depth_texture: Option<metal::Texture>,
    depth_size: (u64, u64),
    sampler: metal::SamplerState,
    shadow_sampler: metal::SamplerState,
    dummy_shadow: metal::Texture,
    flat_normal: metal::Texture,
    white: metal::Texture,
    meshes: Vec<GltfMesh>,
    camera_position: [f32; 3],
    camera_yaw: f32,
    camera_pitch: f32,
    stats: stats::FramePhaseStats,
    last_command_buffer: Option<metal::CommandBuffer>,
}

impl RustMetal {
    unsafe fn new(view: *mut Object) -> Self {
        let device = Device::system_default().expect("no Metal device");
        let raw_layer: *mut Object = unsafe { msg_send![view, layer] };
        let retained: *mut Object = unsafe { msg_send![raw_layer, retain] };
        let layer = unsafe { MetalLayer::from_ptr(retained as *mut CAMetalLayer) };
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        let bounds: CGRect = unsafe { msg_send![view, bounds] };
        let scale = layer.contents_scale();
        layer.set_drawable_size(CGSize::new(bounds.size.width * scale, bounds.size.height * scale));

        let queue = device.new_command_queue();

        let features = ShaderFeatures {
            // This rig has no shadow pass, so the depth texture it would sample
            // is never written and reads as undefined — which darkened the
            // whole scene against the ECS rig, where shadow quality 0 compiles
            // the sampling out. Off by default here for the same reason.
            shadows: env_flag("GFX_SHADOWS"),
            normal_map: !env_flag("GFX_NO_NORMAL_MAP"),
            specular: !env_flag("GFX_NO_SPECULAR"),
            point_light: !env_flag("GFX_NO_POINT_LIGHT"),
            flat: env_flag("GFX_FLAT"),
        };
        let source = format!("{}{SPONZA_SHADER_SOURCE}", features.defines());
        let library = device
            .new_library_with_source(&source, &CompileOptions::new())
            .expect("scene shader failed to compile");

        // Two pipelines and two depth states, matching what the shared renderer
        // does: opaque with blending off and depth writes on, transparent with
        // blending on and depth writes off so it cannot occlude what follows.
        let build_pipeline = |blending: bool| {
            let descriptor = RenderPipelineDescriptor::new();
            descriptor.set_vertex_function(Some(&library.get_function("sponza_vertex", None).unwrap()));
            descriptor.set_fragment_function(Some(&library.get_function("sponza_fragment", None).unwrap()));
            let color = descriptor.color_attachments().object_at(0).unwrap();
            color.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
            color.set_blending_enabled(blending);
            if blending {
                color.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
                color.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
                color.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
                color.set_source_alpha_blend_factor(metal::MTLBlendFactor::SourceAlpha);
                color.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
                color.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
            }
            descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
            device.new_render_pipeline_state(&descriptor).expect("pipeline")
        };
        let pipeline = build_pipeline(false);
        let blend_pipeline = build_pipeline(true);

        let depth_descriptor = metal::DepthStencilDescriptor::new();
        depth_descriptor.set_depth_compare_function(metal::MTLCompareFunction::Less);
        depth_descriptor.set_depth_write_enabled(true);
        let depth_state = device.new_depth_stencil_state(&depth_descriptor);

        let blend_depth_descriptor = metal::DepthStencilDescriptor::new();
        blend_depth_descriptor.set_depth_compare_function(metal::MTLCompareFunction::Less);
        blend_depth_descriptor.set_depth_write_enabled(false);
        let blend_depth_state = device.new_depth_stencil_state(&blend_depth_descriptor);

        let sampler_descriptor = metal::SamplerDescriptor::new();
        sampler_descriptor.set_min_filter(metal::MTLSamplerMinMagFilter::Linear);
        sampler_descriptor.set_mag_filter(metal::MTLSamplerMinMagFilter::Linear);
        if !env_flag("GFX_NO_MIPS") {
            sampler_descriptor.set_mip_filter(metal::MTLSamplerMipFilter::Linear);
        }
        sampler_descriptor.set_address_mode_s(metal::MTLSamplerAddressMode::Repeat);
        sampler_descriptor.set_address_mode_t(metal::MTLSamplerAddressMode::Repeat);
        let sampler = device.new_sampler(&sampler_descriptor);

        let shadow_descriptor = metal::SamplerDescriptor::new();
        shadow_descriptor.set_compare_function(metal::MTLCompareFunction::LessEqual);
        let shadow_sampler = device.new_sampler(&shadow_descriptor);

        let shadow_texture_descriptor = metal::TextureDescriptor::new();
        shadow_texture_descriptor.set_pixel_format(MTLPixelFormat::Depth32Float);
        shadow_texture_descriptor.set_width(16);
        shadow_texture_descriptor.set_height(16);
        shadow_texture_descriptor.set_usage(metal::MTLTextureUsage::ShaderRead | metal::MTLTextureUsage::RenderTarget);
        shadow_texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        let dummy_shadow = device.new_texture(&shadow_texture_descriptor);

        let flat_normal = metal_renderer::metal_texture::upload_texture(&device, 1, 1, &[128, 128, 255, 255]);
        let white = metal_renderer::metal_texture::upload_texture(&device, 1, 1, &[255, 255, 255, 255]);

        let resource_path = bundle_resource_path().unwrap_or_default();
        let path = std::path::Path::new(&resource_path).join(MAP_PATH);
        let meshes = load_gltf_scene(&device, &path);

        Self {
            device,
            queue,
            layer,
            pipeline,
            blend_pipeline,
            depth_state,
            blend_depth_state,
            depth_texture: None,
            depth_size: (0, 0),
            sampler,
            shadow_sampler,
            dummy_shadow,
            flat_normal,
            white,
            meshes,
            camera_position: [0.0, 3.0, 0.0],
            camera_yaw: 0.0,
            camera_pitch: 0.0,
            stats: stats::FramePhaseStats::default(),
            last_command_buffer: None,
        }
    }

    fn update_camera(&mut self, forward: f32, strafe: f32, look_dx: f32, look_dy: f32, dt: f32) {
        self.camera_yaw -= look_dx * 0.005;
        self.camera_pitch = (self.camera_pitch - look_dy * 0.005).clamp(-1.54, 1.54);
        if forward != 0.0 || strafe != 0.0 {
            let (sin_yaw, cos_yaw) = self.camera_yaw.sin_cos();
            let speed = 6.0 * dt;
            self.camera_position[0] += (-sin_yaw * forward + cos_yaw * strafe) * speed;
            self.camera_position[2] += (-cos_yaw * forward - sin_yaw * strafe) * speed;
        }
    }

    fn ensure_depth(&mut self, width: u64, height: u64) -> metal::Texture {
        if self.depth_size != (width, height) || self.depth_texture.is_none() {
            let descriptor = metal::TextureDescriptor::new();
            descriptor.set_pixel_format(MTLPixelFormat::Depth32Float);
            descriptor.set_width(width);
            descriptor.set_height(height);
            descriptor.set_usage(metal::MTLTextureUsage::RenderTarget);
            descriptor.set_storage_mode(metal::MTLStorageMode::Private);
            self.depth_texture = Some(self.device.new_texture(&descriptor));
            self.depth_size = (width, height);
        }
        self.depth_texture.clone().unwrap()
    }

    fn render_frame(&mut self) {
        let frame_start = std::time::Instant::now();

        let drawable_size = self.layer.drawable_size();
        let (width, height) = (drawable_size.width as u64, drawable_size.height as u64);
        let depth_texture = self.ensure_depth(width, height);
        let Some(drawable) = self.layer.next_drawable() else { return };

        // The same measurement the ECS rig takes, so the two GPU numbers are
        // comparable. Read after next_drawable: before the display hands one
        // back the GPU may not have retired the frame, and the timestamps are
        // zero until it has.
        let mut gpu_ms = 0.0;
        if let Some(previous) = self.last_command_buffer.take() {
            let raw = previous.as_ptr() as *mut Object;
            let start: f64 = unsafe { msg_send![raw, GPUStartTime] };
            let end: f64 = unsafe { msg_send![raw, GPUEndTime] };
            if end > start {
                gpu_ms = ((end - start) * 1000.0) as f32;
            }
        }

        let encode_start = std::time::Instant::now();

        let (sin_yaw, cos_yaw) = self.camera_yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.camera_pitch.sin_cos();
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let centre = [
            self.camera_position[0] + forward[0],
            self.camera_position[1] + forward[1],
            self.camera_position[2] + forward[2],
        ];
        let view = look_at(self.camera_position, centre, [0.0, 1.0, 0.0]);
        // 60 degrees and a 0.05 near plane, matching what the shared renderer
        // gives desert_rust_bevy. Neither rig frustum-culls, so this does not
        // change how many vertices are processed, but it does change how much
        // of the screen they cover.
        let projection = perspective(
            60.0_f32.to_radians(),
            width as f32 / height.max(1) as f32,
            0.05,
            500.0,
        );
        let view_projection = metal_renderer::metal_math::mat_mul(projection, view);

        let descriptor = RenderPassDescriptor::new();
        let color = descriptor.color_attachments().object_at(0).unwrap();
        color.set_texture(Some(drawable.texture()));
        color.set_load_action(MTLLoadAction::Clear);
        color.set_store_action(MTLStoreAction::Store);
        color.set_clear_color(MTLClearColor::new(0.52, 0.62, 0.74, 1.0));
        let depth = descriptor.depth_attachment().unwrap();
        depth.set_texture(Some(&depth_texture));
        depth.set_load_action(MTLLoadAction::Clear);
        depth.set_store_action(MTLStoreAction::DontCare);
        depth.set_clear_depth(1.0);

        let command_buffer = self.queue.new_command_buffer();
        let encoder = command_buffer.new_render_command_encoder(descriptor);
        encoder.set_render_pipeline_state(&self.pipeline);
        encoder.set_depth_stencil_state(&self.depth_state);
        encoder.set_fragment_sampler_state(0, Some(&self.sampler));
        encoder.set_fragment_sampler_state(1, Some(&self.shadow_sampler));
        encoder.set_fragment_texture(1, Some(&self.dummy_shadow));
        encoder.set_fragment_texture(2, Some(&self.dummy_shadow));
        encoder.set_fragment_texture(3, Some(&self.flat_normal));

        // Opaque first with depth writes, then the transparent ones back to
        // front with writes off — the order the shared renderer uses. Drawing
        // everything opaque, as this rig used to, is a different frame.
        let mut blended: Vec<&GltfMesh> = Vec::new();
        for mesh in &self.meshes {
            if mesh.is_blend {
                blended.push(mesh);
            }
        }
        blended.sort_by(|a, b| {
            let distance = |m: &GltfMesh| {
                let d = [
                    m.centre[0] - self.camera_position[0],
                    m.centre[1] - self.camera_position[1],
                    m.centre[2] - self.camera_position[2],
                ];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            };
            distance(b).total_cmp(&distance(a))
        });

        for mesh in self.meshes.iter().filter(|m| !m.is_blend).chain(blended.iter().copied()) {
            if mesh.is_blend {
                encoder.set_render_pipeline_state(&self.blend_pipeline);
                encoder.set_depth_stencil_state(&self.blend_depth_state);
            }
            let uniforms = SponzaUniforms {
                view_projection,
                light_pos: [0.0, 0.0, 0.0],
                light_range: 0.0,
                light_color: [0.0, 0.0, 0.0, 0.0],
                sun_dir: SUN_DIRECTION,
                _sun_pad: 0.0,
                // White, because the ECS rig's DirectionalLight is white and
                // its extractor passes the colour through unchanged. A warmer
                // tint here made the two rigs render different images.
                sun_color: [1.0, 1.0, 1.0, 1.0],
                base_color_factor: mesh.base_color_factor,
                emissive: mesh.emissive,
                light_view_projection: identity(),
                point_light_view_projection: identity(),
                camera_pos: self.camera_position,
                _camera_pad: 0.0,
                model_matrix: identity(),
                normal_matrix: identity(),
            };
            encoder.set_vertex_buffer(0, Some(&mesh.vertex_buffer), 0);
            encoder.set_vertex_bytes(
                1,
                std::mem::size_of::<SponzaUniforms>() as u64,
                &uniforms as *const _ as *const _,
            );
            encoder.set_fragment_bytes(
                1,
                std::mem::size_of::<SponzaUniforms>() as u64,
                &uniforms as *const _ as *const _,
            );
            let texture = mesh.base_color_texture.as_ref().unwrap_or(&self.white);
            encoder.set_fragment_texture(0, Some(texture));
            encoder.draw_indexed_primitives(
                MTLPrimitiveType::Triangle,
                mesh.index_count as u64,
                MTLIndexType::UInt32,
                &mesh.index_buffer,
                0,
            );
        }

        encoder.end_encoding();
        command_buffer.present_drawable(drawable);
        command_buffer.commit();
        self.last_command_buffer = Some(command_buffer.to_owned());

        let encode_ms = encode_start.elapsed().as_secs_f32() * 1000.0;
        self.stats.record(
            frame_start.elapsed().as_secs_f32() * 1000.0,
            encode_ms,
            self.meshes.len(),
            gpu_ms,
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn baseline_create(view: *mut Object) -> *mut std::ffi::c_void {
    let baseline = unsafe { RustMetal::new(view) };
    Box::into_raw(Box::new(baseline)) as *mut std::ffi::c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn baseline_render_frame(obj: *mut std::ffi::c_void) {
    let baseline = unsafe { &mut *(obj as *mut RustMetal) };
    baseline.render_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn baseline_update_camera(
    obj: *mut std::ffi::c_void,
    forward: f32,
    strafe: f32,
    look_dx: f32,
    look_dy: f32,
    delta_time: f32,
) {
    let baseline = unsafe { &mut *(obj as *mut RustMetal) };
    baseline.update_camera(forward, strafe, look_dx, look_dy, delta_time);
}

#[unsafe(no_mangle)]
pub extern "C" fn baseline_release(obj: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(obj as *mut RustMetal) });
}

const _: () = {
    let _ = MTLResourceOptions::StorageModeShared;
};
