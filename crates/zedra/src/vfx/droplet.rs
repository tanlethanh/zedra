use super::{DropletState, SharedDropletState, TRAIL_LEN};
use gpui_ios::{MetalEffectContext, MetalRenderEffect};
use metal::{
    CompileOptions, MTLBlendFactor, MTLBlendOperation, MTLOrigin, MTLPrimitiveType, MTLSize,
    MTLTextureUsage,
};

const DROPLET_SHADER: &str = include_str!("droplet.metal");
// Padding around each blob, in radii; covers blob size, refraction offset, and AA.
const GRAB_PADDING: f32 = 2.0;
// Bucket grab-texture dims so per-frame bbox changes don't reallocate the texture.
const GRAB_BUCKET: u64 = 128;

#[repr(C)]
struct DropletUniforms {
    viewport_size: [f32; 2],
    bbox_origin: [f32; 2],
    bbox_size: [f32; 2],
    grab_size: [f32; 2],
    center: [f32; 2],
    radius: f32,
    _pad: f32,
    trail: [[f32; 2]; TRAIL_LEN],
    // float4 in MSL; offset must stay 16-aligned (pad if fields change).
    _pad2: [f32; 2],
    base_color: [f32; 4],
}

pub struct DropletEffect {
    state: SharedDropletState,
    pipeline: Option<metal::RenderPipelineState>,
    pipeline_format: metal::MTLPixelFormat,
    grab_texture: Option<metal::Texture>,
    compile_failed: bool,
}

impl DropletEffect {
    pub fn new(state: SharedDropletState) -> Self {
        Self {
            state,
            pipeline: None,
            pipeline_format: metal::MTLPixelFormat::Invalid,
            grab_texture: None,
            compile_failed: false,
        }
    }

    fn ensure_pipeline(
        &mut self,
        device: &metal::DeviceRef,
        format: metal::MTLPixelFormat,
    ) -> Option<&metal::RenderPipelineState> {
        if self.compile_failed {
            return None;
        }
        if self.pipeline.is_none() || self.pipeline_format != format {
            match build_pipeline(device, format) {
                Ok(pipeline) => {
                    self.pipeline = Some(pipeline);
                    self.pipeline_format = format;
                }
                Err(error) => {
                    tracing::error!("vfx: droplet pipeline compile failed: {error}");
                    self.compile_failed = true;
                    return None;
                }
            }
        }
        self.pipeline.as_ref()
    }

    fn ensure_grab_texture(
        &mut self,
        device: &metal::DeviceRef,
        format: metal::MTLPixelFormat,
        width: u64,
        height: u64,
    ) -> &metal::TextureRef {
        let stale = self.grab_texture.as_ref().is_none_or(|texture| {
            texture.width() < width || texture.height() < height || texture.pixel_format() != format
        });
        if stale {
            let descriptor = metal::TextureDescriptor::new();
            descriptor.set_pixel_format(format);
            descriptor.set_width(width.div_ceil(GRAB_BUCKET) * GRAB_BUCKET);
            descriptor.set_height(height.div_ceil(GRAB_BUCKET) * GRAB_BUCKET);
            descriptor.set_usage(MTLTextureUsage::ShaderRead);
            self.grab_texture = Some(device.new_texture(&descriptor));
        }
        self.grab_texture
            .as_ref()
            .expect("grab texture was created above")
    }
}

impl MetalRenderEffect for DropletEffect {
    fn encode(&mut self, cx: &MetalEffectContext) {
        let snapshot: DropletState = match self.state.lock() {
            Ok(state) => *state,
            Err(_) => return,
        };
        if !snapshot.active || snapshot.radius <= 0.0 {
            return;
        }

        let scale = if snapshot.scale_factor > 0.0 {
            snapshot.scale_factor
        } else {
            1.0
        };
        let viewport_width = cx.viewport_size.width.0 as f32;
        let viewport_height = cx.viewport_size.height.0 as f32;
        let center = (snapshot.center.0 * scale, snapshot.center.1 * scale);
        let radius = snapshot.radius * scale;
        let trail = snapshot.trail.map(|(x, y)| [x * scale, y * scale]);

        // Bounding box over the head and every trail blob.
        let padding = radius * GRAB_PADDING;
        let mut min_x = center.0;
        let mut min_y = center.1;
        let mut max_x = center.0;
        let mut max_y = center.1;
        for [x, y] in trail {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let left = (min_x - padding).max(0.0).floor();
        let top = (min_y - padding).max(0.0).floor();
        let right = (max_x + padding).min(viewport_width).ceil();
        let bottom = (max_y + padding).min(viewport_height).ceil();
        let bbox_width = (right - left) as u64;
        let bbox_height = (bottom - top) as u64;
        if bbox_width < 2 || bbox_height < 2 {
            return;
        }

        let format = cx.drawable_texture.pixel_format();
        if self.ensure_pipeline(cx.device, format).is_none() {
            return;
        }

        let grab_texture = self
            .ensure_grab_texture(cx.device, format, bbox_width, bbox_height)
            .to_owned();
        let blit_encoder = cx.command_buffer.new_blit_command_encoder();
        blit_encoder.copy_from_texture(
            cx.drawable_texture,
            0,
            0,
            MTLOrigin {
                x: left as u64,
                y: top as u64,
                z: 0,
            },
            MTLSize {
                width: bbox_width,
                height: bbox_height,
                depth: 1,
            },
            &grab_texture,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
        );
        blit_encoder.end_encoding();

        let uniforms = DropletUniforms {
            viewport_size: [viewport_width, viewport_height],
            bbox_origin: [left, top],
            bbox_size: [bbox_width as f32, bbox_height as f32],
            grab_size: [grab_texture.width() as f32, grab_texture.height() as f32],
            center: [center.0, center.1],
            radius,
            _pad: 0.0,
            trail,
            _pad2: [0.0, 0.0],
            base_color: [
                snapshot.base_color.0,
                snapshot.base_color.1,
                snapshot.base_color.2,
                0.0,
            ],
        };

        let render_pass = metal::RenderPassDescriptor::new();
        let Some(color_attachment) = render_pass.color_attachments().object_at(0) else {
            return;
        };
        color_attachment.set_texture(Some(cx.drawable_texture));
        color_attachment.set_load_action(metal::MTLLoadAction::Load);
        color_attachment.set_store_action(metal::MTLStoreAction::Store);

        // ensure_pipeline succeeded above, so the pipeline is present.
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let encoder = cx.command_buffer.new_render_command_encoder(render_pass);
        encoder.set_render_pipeline_state(pipeline);
        let uniforms_ptr = std::ptr::from_ref(&uniforms).cast();
        let uniforms_len = std::mem::size_of::<DropletUniforms>() as u64;
        encoder.set_vertex_bytes(0, uniforms_len, uniforms_ptr);
        encoder.set_fragment_bytes(0, uniforms_len, uniforms_ptr);
        encoder.set_fragment_texture(0, Some(&grab_texture));
        encoder.draw_primitives(MTLPrimitiveType::TriangleStrip, 0, 4);
        encoder.end_encoding();
    }
}

fn build_pipeline(
    device: &metal::DeviceRef,
    format: metal::MTLPixelFormat,
) -> anyhow::Result<metal::RenderPipelineState> {
    // Inject TRAIL_LEN so the MSL array length can't drift from DropletUniforms.
    let source = format!("#define TRAIL_LEN {TRAIL_LEN}u\n{DROPLET_SHADER}");
    let library = device
        .new_library_with_source(&source, &CompileOptions::new())
        .map_err(|error| anyhow::anyhow!("shader compile: {error}"))?;
    let vertex_function = library
        .get_function("droplet_vertex", None)
        .map_err(|error| anyhow::anyhow!("vertex fn: {error}"))?;
    let fragment_function = library
        .get_function("droplet_fragment", None)
        .map_err(|error| anyhow::anyhow!("fragment fn: {error}"))?;

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_vertex_function(Some(&vertex_function));
    descriptor.set_fragment_function(Some(&fragment_function));
    let color_attachment = descriptor
        .color_attachments()
        .object_at(0)
        .ok_or_else(|| anyhow::anyhow!("missing color attachment slot"))?;
    color_attachment.set_pixel_format(format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(MTLBlendFactor::SourceAlpha);
    color_attachment.set_destination_rgb_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_source_alpha_blend_factor(MTLBlendFactor::One);
    color_attachment.set_destination_alpha_blend_factor(MTLBlendFactor::OneMinusSourceAlpha);

    device
        .new_render_pipeline_state(&descriptor)
        .map_err(|error| anyhow::anyhow!("pipeline state: {error}"))
}
