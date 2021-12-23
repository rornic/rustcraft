use std::io::BufReader;

use crate::util;
use glium::texture::{SrgbTexture2d, TextureCreationError};
use glium::Display;
use image::{ImageError, ImageFormat};

#[derive(Debug)]
pub enum TextureLoadError {
    IoError(std::io::Error),
    ImageError(ImageError),
    TextureCreationError(TextureCreationError),
}

/// Loads a texture from a file
pub fn load_texture(display: &Display, file: &str) -> Result<SrgbTexture2d, TextureLoadError> {
    // Read image in from file
    let file = util::get_resource_file(file).map_err(|e| TextureLoadError::IoError(e))?;
    let reader = BufReader::new(file);
    let image = image::load(reader, ImageFormat::Png)
        .map_err(|e| TextureLoadError::ImageError(e))?
        .to_rgba8();

    // Load image onto GPU
    let dimensions = image.dimensions();
    let image = glium::texture::RawImage2d::from_raw_rgba_reversed(&image.into_raw(), dimensions);
    SrgbTexture2d::new(display, image).map_err(|e| TextureLoadError::TextureCreationError(e))
}
