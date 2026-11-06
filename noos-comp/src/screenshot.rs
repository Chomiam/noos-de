//! Capture d'écran : copie du framebuffer rendu vers un PNG.

use std::path::Path;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, gles::GlesRenderer},
    },
    utils::{Physical, Rectangle, Size},
};
use tracing::{info, warn};

pub fn capture(
    renderer: &mut GlesRenderer,
    fb: &<GlesRenderer as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: Size<i32, Physical>,
    flip_y: bool,
    path: &Path,
) {
    let region = Rectangle::from_size((size.w, size.h).into());
    let result = renderer
        .copy_framebuffer(fb, region, Fourcc::Abgr8888)
        .and_then(|mapping| renderer.map_texture(&mapping).map(|b| b.to_vec()));
    match result {
        Ok(mut pixels) => {
            if flip_y {
                // OpenGL lit les lignes de bas en haut.
                let stride = size.w as usize * 4;
                let rows: Vec<Vec<u8>> = pixels.chunks(stride).map(|r| r.to_vec()).rev().collect();
                pixels = rows.concat();
            }
            if let Err(e) = write_png(path, size.w as u32, size.h as u32, &pixels) {
                warn!("capture {} : {e}", path.display());
            } else {
                info!(path = %path.display(), "capture d'écran enregistrée");
            }
        }
        Err(e) => warn!("capture impossible : {e:?}"),
    }
}

pub fn write_rgba(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    match write_png(path, w, h, rgba) {
        Ok(()) => info!(path = %path.display(), "capture d'écran enregistrée"),
        Err(e) => warn!("capture {} : {e}", path.display()),
    }
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
