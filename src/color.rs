// Color management: ICC conversion of decoded images to the screen color space

use std::sync::Arc;

use bevy::prelude::*;
use image::{ColorType, DynamicImage, ImageBuffer, Pixel};
use moxcms::{CmsError, ColorProfile, DataColorSpace, Layout, TransformExecutor, TransformOptions};

use crate::ScreenColorSpace;

// MARK: Color Management
// Decoded images are converted from their color space (embedded ICC profile,
// or sRGB when untagged) to the configured screen color space, so all images
// are compared in the same space and displayed with the intended colors.

pub fn convert_to_screen_space(
    mut image: DynamicImage,
    icc: Option<&[u8]>,
    target: ScreenColorSpace,
    path: &str,
) -> DynamicImage {
    let src_profile = match icc {
        Some(icc) => match ColorProfile::new_from_slice(icc) {
            Ok(profile) => profile,
            Err(e) => {
                println!("{path}: ignoring unreadable ICC profile: {e}");
                return image;
            }
        },
        // Untagged images are assumed sRGB: already in place for an sRGB
        // screen, otherwise they need the conversion like everything else.
        None if target == ScreenColorSpace::Srgb => return image,
        None => ColorProfile::new_srgb(),
    };

    let is_gray = matches!(
        image.color(),
        ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16
    );
    // The decoded buffers are RGB or grayscale; a profile for another space
    // (e.g. CMYK, the decoder already converted those pixels) can't apply.
    let expected_space = if is_gray {
        DataColorSpace::Gray
    } else {
        DataColorSpace::Rgb
    };
    if src_profile.color_space != expected_space {
        println!(
            "{path}: ICC profile is for {:?} but pixels are {:?}, profile ignored",
            src_profile.color_space,
            image.color()
        );
        return image;
    }

    let dst_profile = target.profile();
    let result = match &mut image {
        DynamicImage::ImageLuma8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLumaA8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgb8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgba8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLuma16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLumaA16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgb16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgba16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        // f32 formats are not loaded by this app
        _ => Ok(()),
    };
    if let Err(e) = result {
        println!("{path}: color space conversion failed, image kept as-is: {e}");
    }
    image
}

// Sample types moxcms can transform; picks the executor matching the bit depth.
trait IccSample: image::Primitive + Default {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<Self> + Send + Sync>, CmsError>;
}

impl IccSample for u8 {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<u8> + Send + Sync>, CmsError> {
        src.create_transform_8bit(layout, dst, layout, TransformOptions::default())
    }
}

impl IccSample for u16 {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<u16> + Send + Sync>, CmsError> {
        src.create_transform_16bit(layout, dst, layout, TransformOptions::default())
    }
}

fn icc_convert_image<P>(
    src: &ColorProfile,
    dst: &ColorProfile,
    buf: &mut ImageBuffer<P, Vec<P::Subpixel>>,
) -> std::result::Result<(), CmsError>
where
    P: Pixel,
    P::Subpixel: IccSample,
{
    let layout = match P::CHANNEL_COUNT {
        1 => Layout::Gray,
        2 => Layout::GrayAlpha,
        3 => Layout::Rgb,
        _ => Layout::Rgba,
    };
    let row_len = buf.width() as usize * P::CHANNEL_COUNT as usize;
    let transform = P::Subpixel::create_transform(src, dst, layout)?;
    transform_rows_in_place(transform.as_ref(), buf, row_len)
}

// The executor needs distinct source and destination slices; going row by row
// through a scratch buffer avoids duplicating the whole image.
fn transform_rows_in_place<V: Copy + Default>(
    transform: &dyn TransformExecutor<V>,
    data: &mut [V],
    row_len: usize,
) -> std::result::Result<(), CmsError> {
    let mut scratch = vec![V::default(); row_len];
    for row in data.chunks_exact_mut(row_len) {
        scratch.copy_from_slice(row);
        transform.transform(&scratch, row)?;
    }
    Ok(())
}
