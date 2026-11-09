//! A small, headless studio for launcher skin previews. It shares the battle's mesh,
//! material, shader and portrait finishing path; no window, terrain or battle is created.

use anyhow::{Context, Result, ensure};
use glam::Mat4;
use ta_formats::{gaf::Gaf, image::Palette, model3do::Model3do};

use crate::{FrameResources, frame_group, model::{ModelId, ModelRenderer, TextureLibrary},
    portrait::{PortraitRequest, PortraitStudio, StudioSettings}, storage_entry, texture_entry, util};

pub struct PreviewModel {
    pub unit: String,
    pub object: String,
    pub model: Model3do,
    /// Finished standing pose from the caller's script sandbox; hidden pieces are zero.
    pub pieces: Option<Vec<Mat4>>,
}

/// Premultiplied sRGB RGBA, ready for an egui colour image (the same convention as portraits).
pub struct PreviewImage {
    pub size: u32,
    pub rgba: Vec<u8>,
}

pub struct UnitStudio {
    device: wgpu::Device,
    queue: wgpu::Queue,
    frame_layout: wgpu::BindGroupLayout,
    frame: wgpu::BindGroup,
    portrait: PortraitStudio,
    models: Option<ModelRenderer>,
    subjects: Vec<(ModelId, Vec<Mat4>)>,
    readback: Option<(u32, wgpu::Buffer)>,
}

impl UnitStudio {
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(crate::gpu::native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: Default::default(), display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower, compatible_surface: None,
            force_fallback_adapter: false, apply_limit_buckets: false,
        })).context("opening the preview graphics adapter")?;
        let supported = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("unit preview device"),
            required_features: adapter.features() & (wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
            required_limits: wgpu::Limits { max_texture_array_layers: supported.max_texture_array_layers.min(2048),
                max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size, max_buffer_size: supported.max_buffer_size, ..Default::default() },
            ..Default::default()
        })).context("opening the preview graphics device")?;
        // This headless path does not request ray queries or create a battle renderer.
        util::set_ray_traced(false);
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview frame layout"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                storage_entry(3), storage_entry(4), texture_entry(5),
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                texture_entry(7), texture_entry(8), texture_entry(9),
            ],
        });
        let portrait = PortraitStudio::with_device(&device);
        let depth = util::Target::new(&device, "preview shadow dummy", 1, 1, util::DEPTH_FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let dummy = util::Target::new(&device, "preview map dummy", 1, 1, wgpu::TextureFormat::Rgba8Unorm, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let comparison = device.create_sampler(&wgpu::SamplerDescriptor { compare: Some(wgpu::CompareFunction::LessEqual), ..Default::default() });
        let lights = device.create_buffer(&wgpu::BufferDescriptor { label: Some("preview light dummy"), size: 128,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let grid = device.create_buffer(&wgpu::BufferDescriptor { label: Some("preview grid dummy"), size: 16,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let frame = frame_group(&device, "preview frame", &frame_layout, &FrameResources {
            frame_buffer: portrait.frame_buffer(), shadow_view: &depth.view, shadow_sampler: &comparison,
            light_buffer: &lights, light_grid_buffer: &grid, los_view: &dummy.view, los_sampler: &sampler,
            occlusion_view: &dummy.view, cloud_view: &dummy.view, ground_view: &dummy.view, rt_views: None,
        });
        Ok(Self { device, queue, frame_layout, frame, portrait, models: None, subjects: Vec::new(), readback: None })
    }

    /// Replaces the bounded scene. The caller keeps its source data for a later skin change.
    pub fn set_skin(&mut self, gafs: &[Gaf], palette: &Palette, skin: &ta_skins::Pack, subjects: &[PreviewModel]) -> Result<()> {
        self.load(gafs, palette, skin, subjects, false)
    }

    /// As [`UnitStudio::set_skin`], for an editor: every unit takes a livery (the skin's, or
    /// Reborn's own colours without one), which [`UnitStudio::set_livery`] then changes from
    /// one picture to the next.
    pub fn set_skin_for_editing(&mut self, gafs: &[Gaf], palette: &Palette, skin: &ta_skins::Pack, subjects: &[PreviewModel]) -> Result<()> {
        self.load(gafs, palette, skin, subjects, true)
    }

    /// The livery the units are painted with (none: Reborn's own): one small write, the models
    /// as they are. Faces painted only by a skin made for editing, or with a livery of its own.
    pub fn set_livery(&mut self, livery: Option<&ta_skins::Livery>) -> Result<()> {
        let models = self.models.as_ref().context("preview models have not been loaded")?;
        models.textures.set_liveries(&self.queue, livery, livery);
        Ok(())
    }

    fn load(&mut self, gafs: &[Gaf], palette: &Palette, skin: &ta_skins::Pack, subjects: &[PreviewModel], editing: bool) -> Result<()> {
        ensure!(!subjects.is_empty() && subjects.len() <= 4, "a preview needs one to four models");
        // Drop the previous material bank before allocating its replacement.
        self.models = None;
        self.subjects.clear();
        // (Only the subjects' textures: a few dozen layers to redraw, not the whole install's.)
        let used: std::collections::HashSet<String> = subjects.iter().flat_map(|s| crate::model::drawn_textures(&s.model)).collect();
        let textures = if editing {
            TextureLibrary::new_for_liveries(&self.device, &self.queue, gafs, palette, Some(skin), Some(&used))
        } else {
            TextureLibrary::new_with_skin(&self.device, &self.queue, gafs, palette, Some(skin), Some(&used))
        };
        let mut models = ModelRenderer::new(&self.device, &self.frame_layout, util::HDR_FORMAT, 1, textures);
        for subject in subjects {
            let id = models.library.add_unit(&subject.unit, &subject.object, &subject.model, &models.textures, palette);
            if let Some(pieces) = &subject.pieces {
                ensure!(pieces.len() == models.library.info(id).pieces.len(), "preview pose must match its model");
            }
            self.subjects.push((id, subject.pieces.clone().unwrap_or_else(|| models.library.info(id).rest_pose())));
        }
        self.models = Some(models);
        Ok(())
    }

    /// Explicit requests only: no internal animation loop or GPU work while the page is idle.
    pub fn render(&mut self, yaw: f32, size: u32) -> Result<Vec<PreviewImage>> {
        let size = size.clamp(128, 384).div_ceil(64) * 64;
        let models = self.models.as_mut().context("preview models have not been loaded")?;
        let rotation = Mat4::from_rotation_y(if yaw.is_finite() { yaw } else { 0.0 });
        let padded = size * 4; // Sizes are multiples of 64: every row is copy-aligned.
        if self.readback.as_ref().is_none_or(|(old, _)| *old != size) {
            self.readback = Some((size, self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("preview readback"), size: padded as u64 * size as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
            })));
        }
        let buffer = &self.readback.as_ref().expect("created above").1;
        let mut images = Vec::with_capacity(self.subjects.len());
        for (i, (model, rest)) in self.subjects.iter().enumerate() {
            let pieces: Vec<_> = rest.iter().map(|m| rotation * *m).collect();
            let picture = self.portrait.render_model(&self.device, &self.queue, models, &self.frame,
                &PortraitRequest { model: *model, pieces: &pieces, team: if i == 0 { 0 } else { 1 }, size },
                StudioSettings { samples: 1, art_reborn: true, colour_blind: false, hd: 1.0, exposure: 1.0, frame_scale: 0.80 })
                .context("the model has no visible surfaces")?;
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("preview readback") });
            encoder.copy_texture_to_buffer(picture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size) } },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 });
            self.queue.submit(Some(encoder.finish()));
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(5)) })
                .context("reading the unit preview")?;
            let bytes = slice.get_mapped_range().context("mapping the unit preview")?;
            images.push(PreviewImage { size, rgba: bytes.to_vec() });
            drop(bytes);
            buffer.unmap();
        }
        Ok(images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit GPU smoke test: validates actual rendered skin/material changes, rotation,
    /// and stable repeated static frames, independently of launcher screenshot timing.
    #[test]
    #[ignore = "requires a GPU and the locally installed original game"]
    fn installed_skin_preview_rotates_and_changes_materials() {
        let root = std::env::var_os("TA_REBORN_GAME_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:/GOG Games/Total Annihilation".into());
        let mut vfs = ta_vfs::Vfs::new(); vfs.mount_game_dir(&root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let gafs: Vec<_> = vfs.list("textures", Some("gaf"), false).iter()
            .filter_map(|p| vfs.read_opt(p).and_then(|b| Gaf::parse(&b).ok())).collect();
        let subjects: Vec<_> = ["ARMSTUMP", "CORRAID"].into_iter().map(|name| PreviewModel {
            unit: name.into(), object: name.into(), pieces: None,
            model: Model3do::parse(&vfs.read(&format!("objects3d/{name}.3do")).unwrap()).unwrap(),
        }).collect();
        let mut studio = UnitStudio::new().unwrap();
        let mut previous: Option<Vec<PreviewImage>> = None;
        let mut pictures = std::collections::BTreeMap::new();
        for id in ta_skins::builtin_ids() {
            let pack = ta_skins::load(id, std::path::Path::new(".")).unwrap();
            studio.set_skin(&gafs, &palette, &pack, &subjects).unwrap();
            let images = studio.render(0.0, 320).unwrap();
            assert_eq!(images.len(), 2);
            for (index, image) in images.iter().enumerate() {
                let visible = image.rgba.chunks_exact(4).filter(|px| px[3] > 128).count();
                assert!(visible > 1000, "{id} model {index} was not rendered");
                if let Some(before) = &previous { assert_ne!(before[index].rgba, image.rgba, "{id}: skin material must change"); }
                if let Some(folder) = std::env::var_os("TA_PREVIEW_SHOTS") {
                    let folder = std::path::PathBuf::from(folder); std::fs::create_dir_all(&folder).unwrap();
                    image::save_buffer(folder.join(format!("{}-{index}.png", id.replace(':', "-"))), &image.rgba,
                        image.size, image.size, image::ColorType::Rgba8).unwrap();
                }
            }
            let still = studio.render(0.0, 320).unwrap();
            assert_eq!(images[0].rgba, still[0].rgba, "static previews must be stable");
            let turned = studio.render(0.65, 320).unwrap();
            assert_ne!(images[0].rgba, turned[0].rgba, "preview must rotate the actual model");
            pictures.insert(id, images.iter().map(|i| i.rgba.clone()).collect::<Vec<_>>());
            previous = Some(images);
        }
        // An editor's models: without a livery they are Reborn's to the pixel; given one, the
        // game's picture of it, with no models made again.
        let reborn = ta_skins::load("reborn", std::path::Path::new(".")).unwrap();
        studio.set_skin_for_editing(&gafs, &palette, &reborn, &subjects).unwrap();
        let same: Vec<_> = studio.render(0.0, 320).unwrap().into_iter().map(|i| i.rgba).collect();
        assert!(same == pictures["reborn"], "an editor's preview without a livery is Reborn's");
        for id in ["builtin:desert", "builtin:ceremonial"] {
            studio.set_livery(ta_skins::builtin_livery(id).as_ref()).unwrap();
            let painted: Vec<_> = studio.render(0.0, 320).unwrap().into_iter().map(|i| i.rgba).collect();
            assert!(painted == pictures[id], "{id} painted live is {id} as loaded");
        }
        studio.set_livery(None).unwrap();
        let back: Vec<_> = studio.render(0.0, 320).unwrap().into_iter().map(|i| i.rgba).collect();
        assert!(back == pictures["reborn"]);
    }
}
