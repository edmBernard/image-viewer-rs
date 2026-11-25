// Keyboard, mouse, scroll and file drop handlers

use std::fs::canonicalize;
use std::time::{Duration, Instant};

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;

use crate::layout::CellGrid;
use crate::{
    AddMode, Config, GlobalRotation, GlobalScale, GridLayoutState, ImageList, LoadImagesEvent, MouseState,
    MoveImageEvent, MultiCursorEnabled, MyAppState, MyImage, Position, RecordedPressedKey, ReviewState, Rotation,
    SaveCropped, Scale, ScrollBehavior, SwitchReviewToFileEvent, UiState, CTRL_KEYS, DIGIT_KEYS, SHIFT_KEYS,
};

// MARK: Keyboard Handlers
pub fn key_change_layout(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    if keys.just_pressed(config.shortcut.switch_layout) {
        layout_state.layout = layout_state.layout.next();
        move_image_evw.write(MoveImageEvent);
    }
}

pub fn change_layout_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut layout_state: ResMut<GridLayoutState>,
    mut click_timer: Local<Option<Instant>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let now = Instant::now();
        let Some(double_click_time) = *click_timer else {
            *click_timer = Some(now);
            return;
        };

        if now > double_click_time + Duration::from_millis(300) {
            *click_timer = Some(now);
            return;
        }
        *click_timer = Some(now);

        layout_state.layout = layout_state.layout.toggle();
        move_image_evw.write(MoveImageEvent);
    }
}

pub fn change_top_image(
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    if keys.any_pressed(CTRL_KEYS) || !keys.any_pressed(SHIFT_KEYS) {
        return;
    }
    let Some(index) = just_pressed_digit(&keys) else {
        return;
    };
    layout_state.index = index;
    move_image_evw.write(MoveImageEvent);
}

pub fn change_global_rotation(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut global_rotation: ResMut<GlobalRotation>,
) {
    if keys.just_pressed(config.shortcut.rotate_images) {
        global_rotation.0 += 1;
        move_image_evw.write(MoveImageEvent);
    };
}

pub fn change_global_zoom(
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut global_scale: ResMut<GlobalScale>,
) {
    if !keys.any_pressed(CTRL_KEYS) {
        return;
    }
    // Only 1 to 5 are zoom shortcuts
    let Some(digit) = just_pressed_digit(&keys).filter(|&digit| digit < 5) else {
        return;
    };
    // Digit n zooms by 2^(n-1), or by 1/2^n with shift
    let power = if keys.any_pressed(SHIFT_KEYS) {
        -(digit as f32 + 1.)
    } else {
        digit as f32
    };
    global_scale.0 = 2f32.powf(power);
    move_image_evw.write(MoveImageEvent);
}

// Index of the digit key just pressed: Digit1 is 0, ..., Digit0 is 9
fn just_pressed_digit(keys: &ButtonInput<KeyCode>) -> Option<usize> {
    DIGIT_KEYS.iter().position(|&key| keys.just_pressed(key))
}

pub fn change_rotation_individually(
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    mut sprite_query: Query<&mut Rotation, With<MyImage>>,
) {
    if !keys.pressed(config.shortcut.local_rotate_modifier) {
        return;
    }
    if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
        return;
    }
    let window = windows.single().unwrap();
    let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);
    let Some(slot) = window.cursor_position().and_then(|pos| grid.slot_at(pos)) else {
        return;
    };
    let Ok(mut rotate) = sprite_query.get_mut(image_list.0[slot]) else {
        return;
    };
    rotate.0 += if buttons.just_pressed(MouseButton::Left) { 1 } else { -1 };
    move_image_evw.write(MoveImageEvent);
}

pub fn change_zoom_individually(
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    mut sprite_query: Query<(&mut Scale, &mut Position), With<MyImage>>,
) {
    if !keys.pressed(config.shortcut.local_zoom_modifier) {
        return;
    }
    if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
        return;
    }
    let window = windows.single().unwrap();
    let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);
    let Some(slot) = window.cursor_position().and_then(|pos| grid.slot_at(pos)) else {
        return;
    };
    let Ok((mut scale, position)) = sprite_query.get_mut(image_list.0[slot]) else {
        return;
    };
    scale.0 *= if buttons.just_pressed(MouseButton::Left) {
        2.
    } else {
        0.5
    };
    let position_normalized = position.0 * scale.0;

    // Reset position for other images to match the one we zoom
    for (scale, mut position) in &mut sprite_query {
        position.0 = position_normalized / scale.0;
    }
    move_image_evw.write(MoveImageEvent);
}

pub fn toggle_help(keys: Res<ButtonInput<KeyCode>>, mut ui_state: ResMut<UiState>) {
    if keys.just_pressed(KeyCode::KeyH) {
        ui_state.visible = !ui_state.visible;
        ui_state.settings_panel_visible = false;
    }
}

pub fn key_toggle_cursor(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    mut cursor_state: ResMut<MultiCursorEnabled>,
) {
    if keys.just_pressed(config.shortcut.switch_cursor) {
        cursor_state.0 = !cursor_state.0;
    }
}

pub fn key_save_cropped(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    mut save_cropped_evw: MessageWriter<SaveCropped>,
) {
    if keys.just_pressed(config.shortcut.save_crop_image) {
        save_cropped_evw.write(SaveCropped);
    }
}

pub fn key_toggle_add_mode(keys: Res<ButtonInput<KeyCode>>, config: Res<Config>, mut add_mode: ResMut<AddMode>) {
    if keys.just_pressed(config.shortcut.add_images) {
        add_mode.0 = !add_mode.0;
    }
}

pub fn record_pressed_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut recorded_key: ResMut<RecordedPressedKey>,
    mut next_state: ResMut<NextState<MyAppState>>,
) {
    match keys.get_pressed().last() {
        Some(&key) => recorded_key.0 = Some(key),
        // Back to Working once the recorded key is released, ui_settings_menu applies it
        None if recorded_key.0.is_some() => next_state.set(MyAppState::Working),
        None => {}
    }
}

// MARK: Mouse & Scroll Input
pub fn scroll_events(
    config: Res<Config>,
    mut scroll_evr: MessageReader<MouseWheel>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut global_scale: ResMut<GlobalScale>,
    mut mouse_state: ResMut<MouseState>,
) {
    for ev in scroll_evr.read() {
        match config.misc.scroll_behavior {
            ScrollBehavior::Zoom => global_scale.0 *= if ev.y > 0. { 1.1 } else { 0.9 },
            ScrollBehavior::Move => mouse_state.delta += Vec2::new(ev.x, ev.y),
            ScrollBehavior::None => continue,
        }
        move_image_evw.write(MoveImageEvent);
    }
}

pub fn mouse_button_input(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut mouse_state: ResMut<MouseState>,
    global_scale: Res<GlobalScale>,
    mut image_query: Query<(&mut Position, &Scale), With<MyImage>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let window = windows.single().unwrap();
        let Some(cursor_position) = window.cursor_position() else {
            return;
        };
        mouse_state.pressed = true;
        mouse_state.origin = cursor_position;
        mouse_state.delta = Vec2::ZERO;
    }
    if buttons.just_released(MouseButton::Left) {
        mouse_state.pressed = false;

        let window = windows.single().unwrap();

        let Some(cursor_position) = window.cursor_position() else {
            // Cursor is outside of the windows
            for (mut position, scale) in &mut image_query {
                position.0 += mouse_state.delta / (scale.0 * global_scale.0);
            }
            mouse_state.delta = Vec2::ZERO;
            return;
        };

        for (mut position, scale) in &mut image_query {
            position.0 += (cursor_position - mouse_state.origin) / (scale.0 * global_scale.0);
        }
        mouse_state.origin = cursor_position;
        mouse_state.delta = Vec2::ZERO;
    }
}

pub fn cursor_move(
    mut cursor_evr: MessageReader<CursorMoved>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut mouse_state: ResMut<MouseState>,
) {
    for ev in cursor_evr.read() {
        if mouse_state.pressed {
            mouse_state.delta = ev.position - mouse_state.origin;
            move_image_evw.write(MoveImageEvent);
        }
    }
}

pub fn file_drop(
    mut dnd_evr: MessageReader<FileDragAndDrop>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    add_mode: Res<AddMode>,
    review_state: Res<ReviewState>,
    mut switch_review_evw: MessageWriter<SwitchReviewToFileEvent>,
) {
    let mut images_filename = Vec::new();
    for ev in dnd_evr.read() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = ev {
            // On Windows, explorer hands us 8.3 short names (FOO~1.PNG) when the full path
            // exceeds MAX_PATH. Canonicalizing resolves them back to the real long names.
            let Ok(resolved_path) = canonicalize(path_buf) else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            let Some(image_absolute) = resolved_path.as_path().to_str() else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            images_filename.push(String::from(image_absolute));
        }
    }
    if images_filename.is_empty() {
        return;
    }

    // In review drag-to-switch mode, a dropped image jumps to the set it belongs to
    // rather than replacing the current batch. Only the first dropped file is used.
    if review_state.enabled && review_state.drag_switch {
        if let Some(first) = images_filename.into_iter().next() {
            switch_review_evw.write(SwitchReviewToFileEvent(first));
        }
        return;
    }

    // In add mode the new images are appended after the current ones
    load_image_evw.write(LoadImagesEvent {
        paths: images_filename,
        replace: !add_mode.0,
    });
}
