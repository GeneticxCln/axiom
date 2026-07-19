//! Font atlas / glyph cache for server-side decoration title text.

use ab_glyph::{Font, FontArc, Glyph, Point, PxScale, ScaleFont};
use anyhow::Result;
use log::info;
use std::collections::HashMap;
use wgpu::{Device, Extent3d, Origin3d, Texture, TextureDimension, TextureFormat, TextureUsages};

#[derive(Debug, Clone)]
pub struct CachedGlyph {
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
}

pub struct GlyphCache {
    texture: Texture,
    atlas_width: u32,
    atlas_height: u32,
    cursor_x: u32,
    cursor_y: u32,
    max_row_height: u32,
    font: FontArc,
    cache: HashMap<u64, CachedGlyph>,
}

impl GlyphCache {
    pub fn new(device: &Device) -> Result<Self> {
        let font_data = load_font_bytes()?;
        let font = FontArc::try_from_vec(font_data)?;
        let (atlas_width, atlas_height) = (512, 512);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: atlas_width,
                height: atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        info!("Created font glyph cache ({atlas_width}x{atlas_height})");
        Ok(Self {
            texture,
            atlas_width,
            atlas_height,
            cursor_x: 1,
            cursor_y: 1,
            max_row_height: 0,
            font,
            cache: HashMap::new(),
        })
    }

    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn format() -> TextureFormat {
        TextureFormat::R8Unorm
    }

    pub fn get_glyph(
        &mut self,
        c: char,
        size_px: f32,
        device: &Device,
        queue: &wgpu::Queue,
    ) -> Result<CachedGlyph> {
        let key = (c as u64) | ((size_px.to_bits() as u64) << 32);
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }
        let gid = self.font.glyph_id(c);
        let scaled = self.font.as_scaled(size_px);
        let glyph = Glyph {
            id: gid,
            scale: PxScale::from(size_px),
            position: Point { x: 0.0, y: 0.0 },
        };
        let outlined = scaled
            .outline_glyph(glyph)
            .ok_or_else(|| anyhow::anyhow!("no outline for U+{:04X}", c as u32))?;
        let advance = scaled.h_advance(gid);
        let bearing_x = scaled.h_side_bearing(gid);
        let b = outlined.px_bounds();
        let gw = b.width().ceil() as u32;
        let gh = b.height().ceil() as u32;
        if self.cursor_x + gw + 1 > self.atlas_width {
            self.cursor_x = 1;
            self.cursor_y += self.max_row_height + 1;
            self.max_row_height = 0;
        }
        if self.cursor_y + gh + 1 > self.atlas_height {
            self.grow_atlas(device, queue);
        }
        let bw = gw.max(1);
        let bh = gh.max(1);
        let mut buf = vec![0u8; (bw * bh) as usize];
        outlined.draw(|px, py, cov| {
            let i = py * bw + px;
            if (i as usize) < buf.len() {
                buf[i as usize] = (cov * 255.0) as u8;
            }
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: self.cursor_x,
                    y: self.cursor_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &buf,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bw),
                rows_per_image: Some(bh),
            },
            Extent3d {
                width: bw,
                height: bh,
                depth_or_array_layers: 1,
            },
        );
        let (ux0, uy0) = (
            self.cursor_x as f32 / self.atlas_width as f32,
            self.cursor_y as f32 / self.atlas_height as f32,
        );
        let (ux1, uy1) = (
            (self.cursor_x + gw) as f32 / self.atlas_width as f32,
            (self.cursor_y + gh) as f32 / self.atlas_height as f32,
        );
        self.cursor_x += gw + 1;
        if gh > self.max_row_height {
            self.max_row_height = gh;
        }
        let cached = CachedGlyph {
            uv_min: [ux0, uy0],
            uv_max: [ux1, uy1],
            width: gw as f32,
            height: gh as f32,
            bearing_x,
            bearing_y: -b.min.y,
            advance,
        };
        self.cache.insert(key, cached.clone());
        Ok(cached)
    }

    pub fn layout_text(
        &mut self,
        text: &str,
        size_px: f32,
        x: f32,
        y: f32,
        device: &Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<TextQuad>> {
        let mut qs = Vec::with_capacity(text.len());
        let mut cx = x;
        for c in text.chars() {
            let g = self.get_glyph(c, size_px, device, queue)?;
            qs.push(TextQuad {
                x: cx + g.bearing_x,
                y: y - g.bearing_y,
                w: g.width,
                h: g.height,
                uv_min: g.uv_min,
                uv_max: g.uv_max,
            });
            cx += g.advance;
        }
        Ok(qs)
    }

    fn grow_atlas(&mut self, device: &Device, queue: &wgpu::Queue) {
        let nw = self.atlas_width * 2;
        let nh = (self.atlas_height * 2).min(4096);
        let nt = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas (grown)"),
            size: Extent3d {
                width: nw,
                height: nh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let st = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Atlas migrate staging"),
            size: (self.atlas_width * self.atlas_height) as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Atlas grow"),
        });
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &st,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas_width),
                    rows_per_image: Some(self.atlas_height),
                },
            },
            Extent3d {
                width: self.atlas_width,
                height: self.atlas_height,
                depth_or_array_layers: 1,
            },
        );
        enc.copy_buffer_to_texture(
            wgpu::ImageCopyBuffer {
                buffer: &st,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas_width),
                    rows_per_image: Some(self.atlas_height),
                },
            },
            wgpu::ImageCopyTexture {
                texture: &nt,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: self.atlas_width,
                height: self.atlas_height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(enc.finish()));
        self.texture = nt;
        self.atlas_width = nw;
        self.atlas_height = nh;
    }
}

#[derive(Debug, Clone)]
pub struct TextQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
}

const DEFAULT_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

fn load_font_bytes() -> Result<Vec<u8>> {
    for path in DEFAULT_FONT_PATHS {
        match std::fs::read(path) {
            Ok(data) => {
                log::info!("Loaded font from {}", path);
                return Ok(data);
            }
            Err(e) => {
                log::debug!("Font path {} not available: {}", path, e);
            }
        }
    }
    anyhow::bail!(
        "No system font found. Tried: {}. Install DejaVu Sans, Noto Sans, or Liberation Sans.",
        DEFAULT_FONT_PATHS.join(", ")
    )
}
