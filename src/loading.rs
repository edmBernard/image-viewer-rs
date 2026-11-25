// Image pipeline: decoding, tiling into textures, spawning, removal, reload and crop export

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat};

use crate::color::convert_to_screen_space;
use crate::review_mode::capture_view;
use crate::{
    get_short_name, ChangeSamplerEvent, Config, FitToScreen, FontHandle, GlobalScale, ImageList, ImageOf, ImagePath,
    ImageSize, ImageTile, LoadImagesEvent, MoveImageEvent, MyHelp, MyImage, MyText, MyTile, NewImageLoadedEvent,
    Position, PreservedView, ReloadImagesEvent, RemoveImageEvent, ReviewState, Rotation, SaveCropped, Scale,
    ScreenColorSpace, TileRect, ViewRect, FALLBACK_MAX_TEXTURE_SIZE,
};

// MARK: Image Loading
pub fn on_remove_image(
    mut remove_evr: MessageReader<RemoveImageEvent>,
    mut commands: Commands,
    title_query: Query<(Entity, &ImageOf), With<MyText>>,
    mut image_list: ResMut<ImageList>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
    mut review_state: ResMut<ReviewState>,
) {
    for ev in remove_evr.read() {
        let target = ev.0;
        let Some(slot) = image_list.slot(target) else {
            continue;
        };
        image_list.0.remove(slot);

        // The image despawns with its tiles; its label is a separate entity (cursors are handled by sync_cursors)
        commands.entity(target).despawn();
        for (entity, owner) in &title_query {
            if owner.0 == target {
                commands.entity(entity).despawn();
            }
        }

        // Review patterns are per cell, they follow their image even while review is off,
        // same as on reorder, so they still line up if review is turned back on
        if slot < review_state.cell_patterns.len() {
            review_state.cell_patterns.remove(slot);
        }
        if slot < review_state.editable_patterns.len() {
            review_state.editable_patterns.remove(slot);
        }

        fit_to_screen_evw.write(FitToScreen);
        move_image_evw.write(MoveImageEvent);
    }
}

// Decode an image from disk and convert it from its own color space to `target`
fn decode_image(path: &str, target: ScreenColorSpace) -> Option<DynamicImage> {
    let Some(f) = File::open(path).ok() else {
        println!("Failed to open file: {}", path);
        return None;
    };
    let Some(format) = ImageFormat::from_path(path).ok() else {
        println!("Failed to deduce image format from path: {}", path);
        return None;
    };

    let mut reader = image::ImageReader::with_format(BufReader::new(f), format);

    // Remove the memory limit on image size we can read
    // This is required to process large images that would otherwise be rejected by the image crate
    reader.no_limits();

    let Some(mut decoder) = reader.into_decoder().ok() else {
        println!("Failed to decode image: {}", path);
        return None;
    };
    let icc_profile = decoder.icc_profile().ok().flatten();
    let Some(image) = DynamicImage::from_decoder(decoder).ok() else {
        println!("Failed to decode image: {}", path);
        return None;
    };
    let icc = icc_profile.as_deref().filter(|icc| !icc.is_empty());
    Some(convert_to_screen_space(image, icc, target, path))
}

pub fn on_load_image(
    mut load_evr: MessageReader<LoadImagesEvent>,
    mut loaded_evw: MessageWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
    config: Res<Config>,
    render_device: Option<Res<RenderDevice>>,
) {
    let max_texture_size = match render_device {
        Some(device) => device.limits().max_texture_dimension_2d,
        None => FALLBACK_MAX_TEXTURE_SIZE,
    };
    let sampler = config.misc.sampler_mode.sampler();
    for ev in load_evr.read() {
        // Only the first image that actually loads replaces the current batch
        let mut replace = ev.replace;
        for path in &ev.paths {
            let Some(image) = decode_image(path, config.color.screen_color_space) else {
                continue;
            };

            let size = Vec2::new(image.width() as f32, image.height() as f32);
            let tile_rects = tile_grid(image.width(), image.height(), max_texture_size);
            // A single tile takes the decoded buffer as is; larger images are copied tile by tile
            let tile_images: Vec<(URect, DynamicImage)> = if tile_rects.len() == 1 {
                vec![(tile_rects[0], image)]
            } else {
                tile_rects
                    .into_iter()
                    .map(|r| (r, image.crop_imm(r.min.x, r.min.y, r.width(), r.height())))
                    .collect()
            };

            let mut tiles = Vec::with_capacity(tile_images.len());
            for (rect, tile_image) in tile_images {
                let Some(mut texture) = to_texture(tile_image) else {
                    break;
                };
                texture.sampler = sampler.clone();
                tiles.push(ImageTile {
                    handle: images.add(texture),
                    rect: rect.as_rect(),
                });
            }
            if tiles.is_empty() {
                continue;
            }
            loaded_evw.write(NewImageLoadedEvent {
                tiles,
                size,
                path: path.clone(),
                replace,
            });
            replace = false;
        }
    }
}

// Split a width x height image into the fewest equally sized tiles whose sides fit in max_size
fn tile_grid(width: u32, height: u32, max_size: u32) -> Vec<URect> {
    let cols = width.div_ceil(max_size).max(1);
    let rows = height.div_ceil(max_size).max(1);
    let tile_size = UVec2::new(width.div_ceil(cols), height.div_ceil(rows));
    let image_size = UVec2::new(width, height);
    let mut rects = Vec::with_capacity((cols * rows) as usize);
    for row in 0..rows {
        for col in 0..cols {
            let min = UVec2::new(col, row) * tile_size;
            let max = (min + tile_size).min(image_size);
            rects.push(URect { min, max });
        }
    }
    rects
}

fn to_texture(image: DynamicImage) -> Option<Image> {
    let usages = RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD;
    match image.color() {
        ColorType::Rgb8 | ColorType::Rgba8 | ColorType::L8 | ColorType::La8 | ColorType::Rgb16 | ColorType::Rgba16 => {
            Some(Image::from_dynamic(image, true, usages))
        }
        ColorType::L16 => Some(Image::from_dynamic(
            DynamicImage::ImageRgb16(image.into_rgb16()),
            true,
            usages,
        )),
        _ => {
            println!("Unsupported image type : image.color(): {:?}", image.color());
            None
        }
    }
}

pub fn on_image_loaded(
    config: Res<Config>,
    mut load_image_evr: MessageReader<NewImageLoadedEvent>,
    mut commands: Commands,
    title_query: Query<Entity, With<MyText>>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
    mut image_list: ResMut<ImageList>,
) {
    let events: Vec<&NewImageLoadedEvent> = load_image_evr.read().collect();
    if events.is_empty() {
        return;
    }
    let font = font_query.single().unwrap();

    // Everything before the last replacing image would be dropped anyway, so clear once and
    // skip those (labels spawned earlier in this frame aren't visible to title_query yet)
    let mut start = 0;
    if let Some(last_replace) = events.iter().rposition(|ev| ev.replace) {
        start = last_replace;
        for &entity in &image_list.0 {
            commands.entity(entity).despawn();
        }
        for entity in &title_query {
            commands.entity(entity).despawn();
        }
        image_list.0.clear();
    }

    for ev in &events[start..] {
        // Start hidden; on_move_image will make it visible after positioning.
        // The image entity carries the view state; each tile child carries one texture.
        let image_entity = commands
            .spawn((
                Transform::default(),
                Visibility::Hidden,
                Scale(1.),
                Position(Vec2::ZERO),
                Rotation(0),
                ImagePath(ev.path.clone()),
                ImageSize(ev.size),
                ViewRect::default(),
                MyImage,
            ))
            .with_children(|parent| {
                for tile in &ev.tiles {
                    parent.spawn((
                        Sprite {
                            image: tile.handle.clone(),
                            ..default()
                        },
                        TileRect(tile.rect),
                        MyTile,
                    ));
                }
            })
            .id();

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            Text::new(short_path),
            TextFont {
                font: font.0.clone().into(),
                font_size: FontSize::Px(config.text.font_size),
                ..default()
            },
            TextColor(config.text.font_color),
            TextLayout {
                justify: Justify::Left,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            ImageOf(image_entity),
            MyText,
        ));

        image_list.0.push(image_entity);
    }

    let mut help_visibility = help_query.single_mut().unwrap();
    *help_visibility = Visibility::Hidden;
}

// MARK: Image Cropping
// insert a suffix to given filename
fn insert_suffix(path: &Path, suffix: &str) -> Option<std::path::PathBuf> {
    let parent = path.parent()?;
    let filename = path.file_stem()?.to_str()?;
    let extension = path.extension()?.to_str()?;
    Some(parent.join(filename.to_owned() + suffix + "." + extension))
}

pub fn save_cropped(
    mut save_cropped_evr: MessageReader<SaveCropped>,
    image_query: Query<(&ImagePath, &ViewRect), With<MyImage>>,
) {
    for _ev in save_cropped_evr.read() {
        for (path, view_rect) in &image_query {
            let Some(rect) = view_rect.0 else {
                println!("Failed to get ROI of the texture: {}", path.0);
                continue;
            };
            let Some(output_path) = insert_suffix(Path::new(&path.0), "_crop") else {
                println!("Failed to create output filename: {}", path.0);
                continue;
            };

            // The crop is saved without a profile and may be opened anywhere,
            // so it is always converted to sRGB, not to the screen color space.
            let Some(image) = decode_image(&path.0, ScreenColorSpace::Srgb) else {
                continue;
            };
            let size = rect.size();
            let crop = image.crop_imm(rect.min.x as u32, rect.min.y as u32, size.x as u32, size.y as u32);

            // RGB8 is the one layout every encoder takes (JPEG has no alpha or 16 bits).
            // The format follows the extension, the same as the input.
            if let Err(e) = crop.to_rgb8().save(&output_path) {
                println!("Failed to write data to file {}: {}", output_path.display(), e);
            }
        }
    }
}

// MARK: Sampler
pub fn change_sampler(
    mut change_sampler_evr: MessageReader<ChangeSamplerEvent>,
    config: Res<Config>,
    sprite_query: Query<&Sprite, With<MyTile>>,
    mut images: ResMut<Assets<Image>>,
) {
    if change_sampler_evr.is_empty() {
        return;
    }
    change_sampler_evr.clear();

    let new_sampler = config.misc.sampler_mode.sampler();

    for sprite in &sprite_query {
        let Some(mut image) = images.get_mut(&sprite.image) else {
            continue;
        };
        image.sampler = new_sampler.clone();
    }
}

// MARK: Reload
// Reload all open images from disk, preserving the current zoom/pan.
// Needed when the screen color space changes: conversion happens at decode
// time, so the pixels already uploaded are in the previous space.
pub fn on_reload_images(
    mut reload_evr: MessageReader<ReloadImagesEvent>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    view_query: Query<(&Scale, &Position), With<MyImage>>,
    path_query: Query<&ImagePath, With<MyImage>>,
) {
    if reload_evr.is_empty() {
        return;
    }
    reload_evr.clear();

    if image_list.0.is_empty() {
        return;
    }

    capture_view(&mut preserved, global_scale.0, &image_list, &view_query);
    let mut paths = Vec::new();
    for &entity in &image_list.0 {
        let Ok(path) = path_query.get(entity) else {
            continue;
        };
        paths.push(path.0.clone());
    }
    load_image_evw.write(LoadImagesEvent { paths, replace: true });
}
