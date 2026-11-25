// Review mode systems; the pure pattern logic lives in review.rs

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::{
    review, ActivateReviewEvent, GlobalScale, ImageList, ImagePath, LoadImagesEvent, MyImage, NavigateReviewEvent,
    Position, PreservedView, RefreshReviewEvent, ReviewState, Scale, SwitchReviewToFileEvent,
};

// MARK: Review Mode

// Snapshot the current zoom/pan of the open images so the next review set reuses it
// instead of auto-fitting. Cleared/overwritten on every set switch.
pub fn capture_view(
    preserved: &mut PreservedView,
    global_scale: f32,
    image_list: &ImageList,
    sprite_query: &Query<(&Scale, &Position), With<MyImage>>,
) {
    preserved.global_scale = global_scale;
    preserved.cells.clear();
    for &entity in &image_list.0 {
        let Ok((scale, position)) = sprite_query.get(entity) else {
            continue;
        };
        preserved.cells.push((scale.0, position.0));
    }
    preserved.pending = !preserved.cells.is_empty();
}

pub fn on_navigate_review(
    mut navigate_evr: MessageReader<NavigateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    sprite_query: Query<(&Scale, &Position), With<MyImage>>,
) {
    for ev in navigate_evr.read() {
        if review_state.radixes.is_empty() {
            continue;
        }
        capture_view(&mut preserved, global_scale.0, &image_list, &sprite_query);
        let count = review_state.radixes.len() as i32;
        let new_index = (review_state.current_index as i32 + ev.0).rem_euclid(count) as usize;
        review_state.current_index = new_index;
        load_image_evw.write(review_state.load_set(&review_state.radixes[new_index]));
    }
}

pub fn on_switch_review_to_file(
    mut switch_evr: MessageReader<SwitchReviewToFileEvent>,
    mut review_state: ResMut<ReviewState>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    sprite_query: Query<(&Scale, &Position), With<MyImage>>,
) {
    for ev in switch_evr.read() {
        let Some(filename) = Path::new(&ev.0).file_name().and_then(|name| name.to_str()) else {
            review_state.error = Some("Cannot read the dropped file name".to_string());
            continue;
        };

        let Some(radix) = review::match_radix(filename, &review_state.cell_patterns) else {
            review_state.error = Some("Dropped image doesn't match the review pattern".to_string());
            continue;
        };

        capture_view(&mut preserved, global_scale.0, &image_list, &sprite_query);

        // Locate the set in the scanned list so the counter stays in sync. If it isn't there
        // (e.g. dropped from outside the reviewed directory) we still load it, leaving the index.
        if let Some(index) = review_state.radixes.iter().position(|r| r == &radix) {
            review_state.current_index = index;
        }

        review_state.error = None;
        load_image_evw.write(review_state.load_set(&radix));
    }
}

pub fn on_activate_review(
    mut activate_evr: MessageReader<ActivateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    image_list: Res<ImageList>,
    image_query: Query<&ImagePath, With<MyImage>>,
) {
    for _ev in activate_evr.read() {
        review_state.error = None;
        // Paths in display order so pattern order matches the cells
        let mut paths = Vec::new();
        for &entity in &image_list.0 {
            let Ok(path) = image_query.get(entity) else {
                continue;
            };
            paths.push(path.0.clone());
        }
        if paths.len() < 2 {
            review_state.error = Some("Review mode needs at least 2 images".to_string());
            continue;
        }

        // Extract directory from the first image path
        let Some(dir) = Path::new(&paths[0]).parent() else {
            review_state.error = Some("Cannot determine directory from image path".to_string());
            continue;
        };

        let path_refs: Vec<&Path> = paths.iter().map(Path::new).collect();

        let Some(result) = review::extract_patterns_from_paths(&path_refs) else {
            review_state.error = Some("No common pattern found in filenames".to_string());
            continue;
        };

        let radixes = review::scan_radixes(dir, &result.cell_patterns);
        let current_index = radixes.iter().position(|r| r == &result.radix).unwrap_or(0);

        review_state.enabled = true;
        review_state.directory = dir.to_string_lossy().to_string();
        review_state.editable_patterns = result.cell_patterns.iter().map(|cp| cp.regex_str.clone()).collect();
        review_state.cell_patterns = result.cell_patterns;
        review_state.radixes = radixes;
        review_state.current_index = current_index;
    }
}

pub fn on_refresh_review(
    mut refresh_evr: MessageReader<RefreshReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
) {
    for _ev in refresh_evr.read() {
        // Rebuild cell_patterns from editable_patterns
        let mut new_patterns = Vec::new();
        for (i, regex_str) in review_state.editable_patterns.iter().enumerate() {
            let old = review_state.cell_patterns.get(i);
            let directory = old.map(|cp| cp.directory.clone()).unwrap_or_default();
            let tail = old.map(|cp| cp.tail.clone()).unwrap_or_default();
            new_patterns.push(review::CellPattern::new(directory, tail, regex_str.clone()));
        }
        review_state.cell_patterns = new_patterns;

        let directory = PathBuf::from(&review_state.directory);
        review_state.radixes = review::scan_radixes(&directory, &review_state.cell_patterns);
        if review_state.current_index >= review_state.radixes.len() {
            review_state.current_index = 0;
        }

        // Reload images for the current radix
        if let Some(radix) = review_state.radixes.get(review_state.current_index) {
            load_image_evw.write(review_state.load_set(radix));
        }
    }
}
