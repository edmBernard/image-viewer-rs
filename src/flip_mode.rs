// FLIP mode Bevy systems: reference selection, background computation of the error maps, display switch

use std::sync::mpsc::channel;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::sync::Mutex;
use std::thread;

use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::window::Monitor;
use bevy::window::OnMonitor;
use bevy::winit::EventLoopProxy;
use bevy::winit::EventLoopProxyWrapper;
use bevy::winit::WinitUserEvent;
use image::DynamicImage;
use image::RgbaImage;

use crate::flip::{
    colorize, flip_errors, pixels_per_degree, preprocess, ColorMap, FlipParams, Preprocessed, DEFAULT_PIXELS_PER_DEGREE,
};
use crate::loading::{decode_image, tile_textures};
use crate::{
    get_short_name, Config, FlipChangedEvent, FlipJob, FlipScore, FlipState, FlipTile, GridLayoutState, ImageList,
    ImageOf, ImagePath, ImageSize, MyImage, MyText, MyTile, RecomputeFlipEvent, ScreenColorSpace,
    SetFlipReferenceEvent, ShortcutKeys, TileRect, ToggleFlipEvent, FALLBACK_MAX_TEXTURE_SIZE,
};

// MARK: Background Computation
// What the background thread sends back for each image
pub struct FlipResult {
    image: Entity,
    // None when the map couldn't be computed
    map: Option<FlipMap>,
}

struct FlipMap {
    // Cut the same way as the image it is the map of
    tiles: Vec<(Rect, Image)>,
    mean: f32,
}

// Everything the thread needs besides the images to compare
struct FlipSettings {
    pixels_per_degree: f32,
    color_map: ColorMap,
    max_texture_size: u32,
    sampler: ImageSampler,
}

// FLIP works on sRGB values, whatever the screen color space images are displayed in
fn load_preprocessed(path: &str, params: &FlipParams) -> Option<Preprocessed> {
    let image = decode_image(path, ScreenColorSpace::Srgb)?;
    let (width, height) = (image.width() as usize, image.height() as usize);
    let pixels = image.into_rgb32f().into_raw();
    Some(preprocess(width, height, pixels, params))
}

fn compute_flip_map(
    reference: &Preprocessed,
    path: &str,
    params: &FlipParams,
    settings: &FlipSettings,
) -> Option<FlipMap> {
    let test = load_preprocessed(path, params)?;
    if (test.width, test.height) != (reference.width, reference.height) {
        println!("FLIP needs images of the same size as the reference: {}", path);
        return None;
    }
    let errors = flip_errors(reference, &test, params);
    drop(test);

    let mut sum = 0f64;
    for &error in &errors {
        sum += error as f64;
    }
    let mean = (sum / errors.len().max(1) as f64) as f32;

    let pixels = colorize(&errors, settings.color_map);
    drop(errors);
    let image = RgbaImage::from_raw(reference.width as u32, reference.height as u32, pixels)?;
    let tiles = tile_textures(
        DynamicImage::ImageRgba8(image),
        settings.max_texture_size,
        &settings.sampler,
    );
    if tiles.is_empty() {
        return None;
    }
    Some(FlipMap { tiles, mean })
}

// Runs on its own thread. The reference is preprocessed once, then each map is sent as soon as it's ready.
fn compute_flip_maps(
    reference_path: String,
    targets: Vec<(Entity, String)>,
    settings: FlipSettings,
    sender: Sender<FlipResult>,
    proxy: EventLoopProxy<WinitUserEvent>,
) {
    let params = FlipParams::new(settings.pixels_per_degree);
    let Some(reference) = load_preprocessed(&reference_path, &params) else {
        println!("Failed to load the FLIP reference: {}", reference_path);
        return;
    };
    for (image, path) in targets {
        let map = compute_flip_map(&reference, &path, &params, &settings);
        // Nobody listens anymore when the job has been dropped
        if sender.send(FlipResult { image, map }).is_err() {
            return;
        }
        // The app only updates on input, wake it up to pick the map
        let _ = proxy.send_event(WinitUserEvent::WakeUp);
    }
}

// MARK: Viewing Condition
// Pixels per degree for the configured screen and distance. Also gives the resolution it used,
// which is None (with the default of the reference implementation) while the monitor isn't known.
pub fn viewing_pixels_per_degree(
    config: &Config,
    window_query: &Query<&OnMonitor, With<Window>>,
    monitor_query: &Query<&Monitor>,
) -> (f32, Option<Vec2>) {
    let resolution = 'block: {
        let Some(on_monitor) = window_query.iter().next() else {
            break 'block None;
        };
        let Ok(monitor) = monitor_query.get(on_monitor.0) else {
            break 'block None;
        };
        // At 1:1 an image pixel takes a logical pixel, which is several physical ones on a HiDPI screen
        let logical = monitor.physical_size().as_vec2() / monitor.scale_factor as f32;
        (logical.x > 0. && logical.y > 0.).then_some(logical)
    };
    let Some(resolution) = resolution else {
        return (DEFAULT_PIXELS_PER_DEGREE, None);
    };
    let value = pixels_per_degree(
        config.flip.screen_diagonal_inches,
        config.flip.viewing_distance_cm,
        (resolution.x, resolution.y),
    );
    (value, Some(resolution))
}

// MARK: Systems
pub fn key_toggle_flip(
    shortcut_keys: Res<ShortcutKeys>,
    config: Res<Config>,
    mut toggle_flip_evw: MessageWriter<ToggleFlipEvent>,
) {
    if shortcut_keys.input.just_pressed(config.shortcut.toggle_flip.clone()) {
        toggle_flip_evw.write(ToggleFlipEvent);
    }
}

// Keeps the FLIP state in line with what the user asked and with the images currently open,
// and starts computing the maps that are missing
pub fn on_flip_request(
    mut toggle_flip_evr: MessageReader<ToggleFlipEvent>,
    mut set_reference_evr: MessageReader<SetFlipReferenceEvent>,
    mut recompute_evr: MessageReader<RecomputeFlipEvent>,
    mut flip_changed_evw: MessageWriter<FlipChangedEvent>,
    mut flip_state: ResMut<FlipState>,
    image_list: Res<ImageList>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
    render_device: Option<Res<RenderDevice>>,
    proxy: Res<EventLoopProxyWrapper>,
    image_query: Query<(&ImagePath, &ImageSize, Option<&FlipScore>), With<MyImage>>,
    added_query: Query<(), Added<MyImage>>,
    mut removed_images: RemovedComponents<MyImage>,
    window_query: Query<&OnMonitor, With<Window>>,
    monitor_query: Query<&Monitor>,
) {
    let set_reference = !set_reference_evr.is_empty();
    set_reference_evr.clear();
    let recompute = !recompute_evr.is_empty();
    recompute_evr.clear();
    let mut toggled = false;
    for _ev in toggle_flip_evr.read() {
        toggled = !toggled;
    }
    let images_changed = !added_query.is_empty() || removed_images.read().count() > 0;
    if !set_reference && !recompute && !toggled && !images_changed {
        return;
    }

    if toggled {
        flip_state.shown = !flip_state.shown;
    }

    // Cell to take a new reference from, if it has to change
    let mut new_reference_slot = None;
    if set_reference {
        new_reference_slot = Some(layout_state.index);
    } else if let Some(reference) = flip_state.reference {
        match image_list.slot(reference) {
            // Follows its image when the list is reordered
            Some(slot) => flip_state.reference_slot = slot,
            // Its image has been replaced (review navigation, reload): the reference stays in the same cell
            None => new_reference_slot = Some(flip_state.reference_slot),
        }
    } else if flip_state.shown {
        new_reference_slot = Some(layout_state.index);
    }
    if let Some(slot) = new_reference_slot {
        flip_state.reference = image_list.0.get(slot).copied();
        flip_state.reference_slot = slot;
        flip_state.generation += 1;
        flip_state.job = None;
    }
    if recompute && new_reference_slot.is_none() {
        // Same reference, but the maps computed so far were made with other settings
        flip_state.generation += 1;
        flip_state.job = None;
    }
    flip_changed_evw.write(FlipChangedEvent);

    // Maps are only computed once they are asked for
    if !flip_state.shown {
        return;
    }
    let Some(reference) = flip_state.reference else {
        return;
    };
    // Images spawned this frame are listed before their components exist, they come back as added next frame
    let Ok((reference_path, reference_size, _)) = image_query.get(reference) else {
        return;
    };

    let mut targets = Vec::new();
    for &entity in &image_list.0 {
        if entity == reference {
            continue;
        }
        let Ok((path, size, score)) = image_query.get(entity) else {
            continue;
        };
        if size.0 != reference_size.0 {
            continue;
        }
        if let Some(score) = score {
            if score.generation == flip_state.generation {
                continue;
            }
        }
        targets.push((entity, path.0.clone()));
    }
    if targets.is_empty() {
        return;
    }
    // The running job may already cover them all
    if let Some(job) = &flip_state.job {
        let mut covered = true;
        for (entity, _) in &targets {
            covered &= job.pending.contains(entity);
        }
        if covered {
            return;
        }
    }

    let settings = FlipSettings {
        pixels_per_degree: viewing_pixels_per_degree(&config, &window_query, &monitor_query).0,
        color_map: config.flip.color_map,
        max_texture_size: match render_device {
            Some(device) => device.limits().max_texture_dimension_2d,
            None => FALLBACK_MAX_TEXTURE_SIZE,
        },
        sampler: config.misc.sampler_mode.sampler(),
    };
    let mut pending = Vec::new();
    for (entity, _) in &targets {
        pending.push(*entity);
    }
    let (sender, receiver) = channel();
    let reference_path = reference_path.0.clone();
    let proxy = (**proxy).clone();
    thread::spawn(move || {
        compute_flip_maps(reference_path, targets, settings, sender, proxy.clone());
        // The sender is gone by now: this wake up lets the app see the job is over, even after a failure
        let _ = proxy.send_event(WinitUserEvent::WakeUp);
    });
    // Replacing a running job drops its receiver, which stops its thread
    flip_state.job = Some(FlipJob {
        receiver: Mutex::new(receiver),
        pending,
    });
}

// Picks the maps the background thread has finished and attaches them to their image
pub fn poll_flip_job(
    mut flip_state: ResMut<FlipState>,
    mut flip_changed_evw: MessageWriter<FlipChangedEvent>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    children_query: Query<&Children, With<MyImage>>,
    tile_query: Query<(&TileRect, &Sprite, Option<&FlipTile>), With<MyTile>>,
) {
    if flip_state.job.is_none() {
        return;
    }
    let generation = flip_state.generation;
    let Some(job) = &mut flip_state.job else {
        return;
    };

    let mut results = Vec::new();
    let mut finished = false;
    {
        let Ok(receiver) = job.receiver.lock() else {
            return;
        };
        loop {
            match receiver.try_recv() {
                Ok(result) => results.push(result),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
    }
    if results.is_empty() && !finished {
        return;
    }

    for result in results {
        job.pending.retain(|&entity| entity != result.image);
        let Some(map) = result.map else {
            continue;
        };
        // The image may have been removed while its map was computed
        let Ok(children) = children_query.get(result.image) else {
            continue;
        };
        let mut map_tiles = map.tiles;
        for child in children.iter() {
            let Ok((tile_rect, sprite, flip_tile)) = tile_query.get(child) else {
                continue;
            };
            let Some(index) = map_tiles.iter().position(|(rect, _)| *rect == tile_rect.0) else {
                continue;
            };
            let (_, texture) = map_tiles.swap_remove(index);
            // With a previous map the sprite may be showing it, the original texture is the one kept aside
            let original = match flip_tile {
                Some(flip_tile) => flip_tile.original.clone(),
                None => sprite.image.clone(),
            };
            commands.entity(child).try_insert(FlipTile {
                generation,
                original,
                flip: images.add(texture),
            });
        }
        commands.entity(result.image).try_insert(FlipScore {
            generation,
            mean: map.mean,
        });
    }

    if finished || job.pending.is_empty() {
        flip_state.job = None;
    }
    flip_changed_evw.write(FlipChangedEvent);
}

// Shows the maps or the original images, and says in the labels what is on screen
pub fn on_flip_changed(
    mut flip_changed_evr: MessageReader<FlipChangedEvent>,
    flip_state: Res<FlipState>,
    mut commands: Commands,
    mut tile_query: Query<(Entity, &FlipTile, &mut Sprite), With<MyTile>>,
    image_query: Query<(Entity, &ImagePath, Option<&FlipScore>), With<MyImage>>,
    mut text_query: Query<(&ImageOf, &mut Text), With<MyText>>,
) {
    if flip_changed_evr.is_empty() {
        return;
    }
    flip_changed_evr.clear();

    for (entity, flip_tile, mut sprite) in &mut tile_query {
        // Maps of a previous reference are dropped
        if flip_tile.generation != flip_state.generation {
            sprite.image = flip_tile.original.clone();
            commands.entity(entity).remove::<FlipTile>();
            continue;
        }
        let handle = if flip_state.shown {
            &flip_tile.flip
        } else {
            &flip_tile.original
        };
        if sprite.image != *handle {
            sprite.image = handle.clone();
        }
    }
    for (entity, _, score) in &image_query {
        let Some(score) = score else {
            continue;
        };
        if score.generation != flip_state.generation {
            commands.entity(entity).remove::<FlipScore>();
        }
    }

    for (owner, mut text) in &mut text_query {
        let Ok((image, path, score)) = image_query.get(owner.0) else {
            continue;
        };
        let name = get_short_name(&path.0).unwrap_or("");
        let mut current_score = None;
        if let Some(score) = score {
            if score.generation == flip_state.generation {
                current_score = Some(score.mean);
            }
        }
        let mut computing = false;
        if let Some(job) = &flip_state.job {
            computing = job.pending.contains(&image);
        }

        let label = if !flip_state.shown {
            name.to_string()
        } else if flip_state.reference == Some(image) {
            format!("{name}\n[FLIP reference]")
        } else if let Some(mean) = current_score {
            format!("{name}\n[FLIP mean: {mean:.4}]")
        } else if computing {
            format!("{name}\n[FLIP computing...]")
        } else {
            format!("{name}\n[no FLIP map]")
        };
        if text.0 != label {
            text.0 = label;
        }
    }
}
