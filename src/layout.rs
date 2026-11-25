// Image placement: cell grid, positioning, labels, cursors, fit and reset

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use bevy::window::WindowResized;

use crate::{
    ChangeTitleStyleEvent, Config, FitToScreen, GlobalRotation, GlobalScale, GridLayout, GridLayoutState, ImageList,
    ImageOf, ImageSize, MouseState, MoveImageEvent, MultiCursorEnabled, MyCursor, MyImage, MyText, MyTile, Position,
    PreservedView, ResetScales, Rotation, Scale, TileRect, ViewRect,
};

// MARK: Layout & Positioning
pub fn on_move_image(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut sprite_position: Query<
        (
            &Position,
            &Scale,
            &Rotation,
            &ImageSize,
            &mut ViewRect,
            &mut Transform,
            &mut Visibility,
            &Children,
        ),
        With<MyImage>,
    >,
    mut tiles: Query<(&TileRect, &mut Sprite, &mut Transform, &mut Visibility), (With<MyTile>, Without<MyImage>)>,
    global_scale: Res<GlobalScale>,
    global_rotation: Res<GlobalRotation>,
    layout_state: Res<GridLayoutState>,
    mouse_state: Res<MouseState>,
    image_list: Res<ImageList>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let window = windows.single().unwrap();
    if window.width() == 0. || window.height() == 0. {
        return;
    }
    let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);
    for (slot, &entity) in image_list.0.iter().enumerate() {
        // Images spawned this frame are listed before their components exist
        let Ok((position, scale, rotation, image_size, mut view_rect, mut transform, mut visibility, children)) =
            sprite_position.get_mut(entity)
        else {
            continue;
        };
        let image_size = image_size.0;

        let cell = grid.cell(slot);
        let cell_size = cell.size();
        transform.translation = grid.to_world(cell.center()).extend(transform.translation.z);
        transform.scale = Vec2::splat(scale.0 * global_scale.0).extend(1.);
        let rotation_total = global_rotation.0 + rotation.0;
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation_total as f32);

        let delta = Vec2::from_angle(-PI / 2. * rotation_total as f32)
            .rotate(position.0 + mouse_state.delta / (scale.0 * global_scale.0));
        let image_crop = Rect::from_center_size(image_size / 2., image_size);
        let rotated_cell_size = if rotation_total % 2 == 0 {
            cell_size
        } else {
            Vec2::new(cell_size.y, cell_size.x)
        };
        let cell_center_area = Rect::from_center_size(
            image_size / 2.,
            (image_size - rotated_cell_size / (scale.0 * global_scale.0)).max(Vec2::ONE),
        );
        let cell = Rect::from_center_size(
            bound(image_size / 2. - delta, cell_center_area),
            (rotated_cell_size - 2.) / (scale.0 * global_scale.0),
        );

        let view = cell.intersect(image_crop);
        view_rect.0 = Some(view);

        // Each tile shows its share of the view, offset from the view center in image pixels.
        // The parent transform applies scale and rotation, so child translations stay in image space.
        for child in children.iter() {
            let Ok((tile, mut sprite, mut tile_transform, mut tile_visibility)) = tiles.get_mut(child) else {
                continue;
            };
            let tile_view = view.intersect(tile.0);
            if tile_view.is_empty() {
                *tile_visibility = Visibility::Hidden;
                continue;
            }
            *tile_visibility = Visibility::Inherited;
            sprite.rect = Some(Rect {
                min: tile_view.min - tile.0.min,
                max: tile_view.max - tile.0.min,
            });
            tile_transform.translation = ((tile_view.center() - view.center()) * Vec2::new(1., -1.)).extend(0.);
        }

        // Make visible after positioning (sprites start hidden to avoid flash at native resolution)
        visibility.set_if_neq(grid.visibility(slot));
    }
}

pub fn on_move_image_title(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&ImageOf, &mut Node, &mut Visibility), With<MyText>>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let window = windows.single().unwrap();
    let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);

    for (owner, mut node, mut visibility) in &mut text_query {
        let Some(slot) = image_list.slot(owner.0) else {
            continue;
        };
        let cell = grid.cell(slot);
        node.top = Val::Px(cell.min.y + 2.);
        node.left = Val::Px(cell.min.x + 5.);
        node.width = Val::Px(cell.width());
        visibility.set_if_neq(grid.visibility(slot));
    }
}

pub fn change_image_title_style(
    mut change_style_evr: MessageReader<ChangeTitleStyleEvent>,
    config: Res<Config>,
    mut text_query: Query<(&mut TextFont, &mut TextColor), With<MyText>>,
) {
    if change_style_evr.is_empty() {
        return;
    }
    change_style_evr.clear();

    for (mut text_font, mut text_color) in &mut text_query {
        *text_color = TextColor(config.text.font_color);
        text_font.font_size = FontSize::Px(config.text.font_size);
    }
}

pub fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&ImageOf, &mut Transform, &mut Visibility), With<MyCursor>>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    config: Res<Config>,
) {
    let window = windows.single().unwrap();
    if window.width() == 0. || window.height() == 0. {
        return;
    }
    let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);

    let cursor_position = window.cursor_position();
    for (owner, mut transform, mut visibility) in &mut cursor_query {
        let Some(slot) = image_list.slot(owner.0) else {
            continue;
        };
        visibility.set_if_neq(grid.visibility(slot));

        let Some(cursor_position) = cursor_position else {
            continue;
        };
        // Same position relative to its own cell as the mouse has in the cell it's in
        let cell = grid.cell(slot);
        let position = cell.min + cursor_position.rem_euclid(cell.size());
        transform.translation = grid.to_world(position).extend(transform.translation.z);
    }
}

pub fn on_resize_system(
    mut resize_evr: MessageReader<WindowResized>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
) {
    for _ in resize_evr.read() {
        move_image_evw.write(MoveImageEvent);
    }
}

// Cells of the current layout, in window coordinates (origin top-left, y down)
pub struct CellGrid {
    window_size: Vec2,
    layout: GridLayout,
    selected: usize,
    num_images: usize,
    grid_width: i32,
}

impl CellGrid {
    pub fn new(window: &Window, layout_state: &GridLayoutState, num_images: usize, grid_width: i32) -> Self {
        CellGrid {
            window_size: window.size(),
            layout: layout_state.layout,
            selected: layout_state.index,
            num_images,
            grid_width,
        }
    }

    pub fn cell(&self, slot: usize) -> Rect {
        let n = self.num_images as f32;
        let size = self.window_size;
        let (min, cell_size) = match self.layout {
            GridLayout::Horizontal => {
                let width = size.x / n;
                (Vec2::new(slot as f32 * width, 0.), Vec2::new(width, size.y))
            }
            GridLayout::Vertical => {
                let height = size.y / n;
                (Vec2::new(0., slot as f32 * height), Vec2::new(size.x, height))
            }
            GridLayout::Stack => (Vec2::ZERO, size),
            GridLayout::Grid => {
                let cols = if self.grid_width == 0 {
                    n.sqrt().ceil()
                } else {
                    self.grid_width as f32
                };
                let rows = (n / cols).ceil();
                let cell_size = size / Vec2::new(cols, rows);
                let col_row = Vec2::new((slot as f32).rem_euclid(cols), (slot as f32 / cols).floor());
                (col_row * cell_size, cell_size)
            }
        };
        Rect::from_corners(min, min + cell_size)
    }

    // Only the selected image is shown in Stack layout
    pub fn visibility(&self, slot: usize) -> Visibility {
        if self.layout == GridLayout::Stack && slot != self.selected {
            Visibility::Hidden
        } else {
            Visibility::Visible
        }
    }

    // Slot whose cell is at `pos`. Stacked cells all cover the window, the visible one wins.
    pub fn slot_at(&self, pos: Vec2) -> Option<usize> {
        if self.layout == GridLayout::Stack {
            return (self.selected < self.num_images).then_some(self.selected);
        }
        (0..self.num_images).find(|&slot| self.cell(slot).contains(pos))
    }

    // Window position to world position (origin at the window center, y up)
    pub fn to_world(&self, pos: Vec2) -> Vec2 {
        Vec2::new(pos.x - self.window_size.x / 2., self.window_size.y / 2. - pos.y)
    }
}

// MARK: Cursors
// MultiCursorEnabled is the source of truth: this is the only system spawning or despawning cursors,
// so they follow the image list whatever happens to it (new batch, add mode, removal)
pub fn sync_cursors(
    mut commands: Commands,
    cursor_query: Query<(Entity, &ImageOf), With<MyCursor>>,
    cursor_state: Res<MultiCursorEnabled>,
    image_list: Res<ImageList>,
) {
    let mut has_cursor = Vec::new();
    for (entity, owner) in &cursor_query {
        if cursor_state.0 && image_list.slot(owner.0).is_some() {
            has_cursor.push(owner.0);
        } else {
            commands.entity(entity).despawn();
        }
    }
    if !cursor_state.0 {
        return;
    }

    for &image_entity in &image_list.0 {
        if has_cursor.contains(&image_entity) {
            continue;
        }
        commands
            .spawn((
                Transform::from_translation(Vec3::new(0., 0., 1.)),
                Visibility::default(),
                ImageOf(image_entity),
                MyCursor,
            ))
            .with_children(|parent| {
                // A crosshair with a gap: 4 bars around the center, the side ones turned a quarter
                let bar_size = 15.;
                let bars = [
                    (Vec2::new(bar_size, 0.), -TAU / 4.),
                    (Vec2::new(-bar_size, 0.), -TAU / 4.),
                    (Vec2::new(0., bar_size), 0.),
                    (Vec2::new(0., -bar_size), 0.),
                ];
                for (offset, angle) in bars {
                    parent.spawn((
                        Sprite {
                            color: Color::srgb(0.75, 0., 0.),
                            custom_size: Some(Vec2::new(bar_size, 4.)),
                            ..default()
                        },
                        Transform::from_translation(offset.extend(1.)).with_rotation(Quat::from_rotation_z(angle)),
                    ));
                }
            });
    }
}

// MARK: Fit & Reset
pub fn reset_scales(
    mut reset_evr: MessageReader<ResetScales>,
    mut global_scale: ResMut<GlobalScale>,
    mut sprite_query: Query<(&mut Scale, &mut Position), With<MyImage>>,
) {
    for _ev in reset_evr.read() {
        global_scale.0 = 1.;
        for (mut scale, mut position) in &mut sprite_query {
            position.0 = Vec2::ZERO;
            scale.0 = 1.0;
        }
    }
}

pub fn on_image_spawned(mut fit_to_screen_evw: MessageWriter<FitToScreen>, sprite_query: Query<(), Added<MyImage>>) {
    if sprite_query.is_empty() {
        return;
    }

    fit_to_screen_evw.write(FitToScreen);
}

pub fn fit_to_screen(
    mut fit_to_screen_evr: MessageReader<FitToScreen>,
    windows: Query<&Window>,
    mut global_scale: ResMut<GlobalScale>,
    mut sprite_query: Query<(&ImageSize, &mut Scale, &mut Position), With<MyImage>>,
    image_list: Res<ImageList>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut preserved: ResMut<PreservedView>,
) {
    for _ev in fit_to_screen_evr.read() {
        let window = windows.single().unwrap();
        if window.width() == 0. || window.height() == 0. {
            continue;
        }

        // When switching review sets we reuse the previous zoom/pan instead of refitting.
        if preserved.pending {
            global_scale.0 = preserved.global_scale;
            for (slot, &entity) in image_list.0.iter().enumerate() {
                let Ok((_size, mut scale, mut position)) = sprite_query.get_mut(entity) else {
                    continue;
                };
                if let Some(&(cell_scale, cell_position)) = preserved.cells.get(slot) {
                    scale.0 = cell_scale;
                    position.0 = cell_position;
                }
            }
            preserved.pending = false;
            move_image_evw.write(MoveImageEvent);
            continue;
        }

        let grid = CellGrid::new(window, &layout_state, image_list.0.len(), config.misc.grid_width);
        let mut first = true;
        for (slot, &entity) in image_list.0.iter().enumerate() {
            let Ok((image_size, mut scale, mut position)) = sprite_query.get_mut(entity) else {
                continue;
            };
            let factor = (grid.cell(slot).size() / image_size.0).min_element();

            if first {
                global_scale.0 = factor;
                first = false;
            }
            scale.0 = factor / global_scale.0;
            position.0 = Vec2::ZERO;
        }
        move_image_evw.write(MoveImageEvent);
    }
}

fn bound(vec: Vec2, rect: Rect) -> Vec2 {
    Vec2::new(vec.x.clamp(rect.min.x, rect.max.x), vec.y.clamp(rect.min.y, rect.max.y))
}
