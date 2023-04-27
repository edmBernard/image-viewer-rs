// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

use std::fs::canonicalize;
use std::path::Path;
use std::time::{Duration, Instant};
// use std::io::Cursor;
use std::io::BufReader;
use std::fs::File;

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, Window, WindowResized};
use clap::Parser;
use image::{ColorType, DynamicImage, ImageFormat};
use std::f32::consts::{PI, TAU};

#[doc(hidden)]
type Result<T> = ::std::result::Result<T, Box<dyn ::std::error::Error>>;

#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    /// Images to show
    images: Vec<String>,
}

const HELP_STRING: &'static str = "
Keyboard Shortcut:
    L: Change Layout (Grid, Stack, Horizontal, Vertical)
    R: Rotate images
    1, 2, 3, ...: Move Image on top
    Ctrl/Cmd + 1, 2, 3, 4, 5: Zoom by 1, 2, 4, 8, 16
    C: Toggle multi cursor
    H: Toggle this help

    Drag and Drop image from files explorer.
";

fn main() -> Result<()> {
    let args = Args::parse();
    let images_filename = check_all_images_exist(&args.images)?;

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Image Viewer 3000".to_string(),
                        resolution: [500., 300.].into(),
                        present_mode: PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(InitialImagesFilename(images_filename))
        .add_startup_system(setup)
        .add_event::<LoadNewImageEvent>()
        .add_event::<NewImageLoadedEvent>()
        .add_event::<MoveImageEvent>()
        .add_event::<ResetVisibilityEvent>()
        .add_system(change_layout)
        .add_system(change_layout_on_click)
        .add_system(change_zoom)
        .add_system(scroll_events)
        .add_system(mouse_button_input)
        .add_system(cursor_events)
        .add_system(file_drop)
        .add_system(change_top_image)
        .add_system(change_rotation_image)
        .add_system(on_reset_visibility)
        .add_system(on_resize_system)
        .add_system(on_image_loaded)
        .add_system(on_move_cursor)
        .add_system(on_move_image)
        .add_system(on_move_image_title)
        .add_system(on_load_image)
        .add_system(toggle_help)
        .add_system(toggle_cursor)
        .run();

    Ok(())
}

#[derive(Resource)]
struct InitialImagesFilename(Vec<String>);

#[derive(Component)]
enum GridLayout {
    Stack,
    Horizontal,
    Vertical,
    Grid,
}

#[derive(Component)]
struct Id(i8);

#[derive(Component)]
struct Scale(Vec2);

#[derive(Component)]
struct Position(Vec2);

/// Rotation in quarter turn (1 is 1 turn)
#[derive(Component)]
struct Rotation(f32);

#[derive(Component)]
struct MyCursor;

#[derive(Component)]
struct MyImage;

#[derive(Component)]
struct MyText;

#[derive(Component)]
struct MyHelp;

#[derive(Component)]
struct MouseState {
    origin: Vec2,
    delta: Vec2,
    pressed: bool,
}

struct MoveImageEvent;

struct ResetVisibilityEvent;

struct NewImageLoadedEvent {
    handle: Handle<Image>,
    path: String,
    index: i8,
    count: i8,
}

#[derive(Component)]
struct TotalImageLoaded(i8);

#[derive(Component)]
struct FontHandle(Handle<Font>);

struct LoadNewImageEvent {
    path: String,
    index: i8,
    count: i8,
}

fn setup(
    mut commands: Commands,
    images_filename: ResMut<InitialImagesFilename>,
    mut load_image_evw: EventWriter<LoadNewImageEvent>,
    mut fonts: ResMut<Assets<Font>>,
) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(GridLayout::Grid);
    commands.spawn(TotalImageLoaded(0));
    commands.spawn(MouseState {
        origin: Vec2::ZERO,
        delta: Vec2::ZERO,
        pressed: false,
    });
    let bytes = include_bytes!("../assets/fonts/IBMPlexMono-Regular.otf");
    let font = Font::try_from_bytes(bytes.to_vec()).unwrap();
    let font_handle = fonts.add(font);
    commands.spawn(FontHandle(font_handle.clone()));

    commands.spawn((
        TextBundle::from_section(
            HELP_STRING,
            TextStyle {
                font: font_handle,
                font_size: 18.0,
                color: Color::ANTIQUE_WHITE,
            },
        )
        .with_text_alignment(TextAlignment::Left)
        .with_style(Style {
            position_type: PositionType::Absolute,
            position: UiRect {
                top: Val::Px(22.),
                left: Val::Px(5.),
                ..default()
            },
            ..default()
        }),
        MyHelp,
    ));

    let count = images_filename.0.len();
    for (index, image) in images_filename.0.iter().enumerate() {
        load_image_evw.send(LoadNewImageEvent {
            path: image.clone(),
            index: index as i8,
            count: count as i8,
        });
    }
}

fn on_load_image(
    mut load_evr: EventReader<LoadNewImageEvent>,
    mut loaded_evw: EventWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
) {
    for ev in load_evr.iter() {
        let Some(f) = File::open(&ev.path).ok() else {
            println!("Failed to open file {}", ev.path);
            continue;
        };
        let Some(format) = ImageFormat::from_path(&ev.path).ok() else {
            println!("Failed to deduce image format from path");
            continue;
        };
        let buf = BufReader::new(f);
        let mut reader = image::io::Reader::with_format(buf, format);
        reader.no_limits();

        let Some(image) = reader.decode().ok() else {
            println!("Failed to decode image");
            continue;
        };

        match image.color() {
            ColorType::Rgb8 | ColorType::Rgba8 => {
                let new_image = Image::from_dynamic(image, true);
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            ColorType::Rgb16 | ColorType::Rgba16 => {
                let image_8u = DynamicImage::ImageRgb8(image.into_rgb8());
                let new_image = Image::from_dynamic(image_8u, true);
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            _ => {
                println!("image.color(): {:?}", image.color())
            }
        }
    }
}

fn on_image_loaded(
    mut load_image_evr: EventReader<NewImageLoadedEvent>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vis_evw: EventWriter<ResetVisibilityEvent>,
    mut commands: Commands,
    images: Query<Entity, With<Id>>,
    mut count_query: Query<&mut TotalImageLoaded>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
) {
    for ev in load_image_evr.iter() {
        let mut already_loaded = count_query.single_mut();
        let font = font_query.single();

        if already_loaded.0 == 0 {
            for entity in &images {
                commands.entity(entity).despawn();
            }
        }
        already_loaded.0 += 1;
        if already_loaded.0 >= ev.count {
            already_loaded.0 = 0;
        }

        commands.spawn((
            SpriteBundle {
                texture: ev.handle.clone(),
                ..default()
            },
            Id(ev.index),
            Scale(Vec2::ONE / 8.),
            Position(Vec2::ZERO),
            Rotation(0.),
            MyImage,
        ));

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            TextBundle::from_section(
                short_path,
                TextStyle {
                    font: font.0.clone(),
                    font_size: 16.0,
                    color: Color::GREEN,
                },
            )
            .with_text_alignment(TextAlignment::Left)
            .with_style(Style {
                position_type: PositionType::Absolute,
                ..default()
            }),
            Id(ev.index),
            MyText,
        ));
        reset_vis_evw.send(ResetVisibilityEvent);
        move_image_evw.send(MoveImageEvent);

        let mut visibility = help_query.single_mut();
        *visibility = Visibility::Hidden;
    }
}

fn on_move_image(
    mut move_image_evr: EventReader<MoveImageEvent>,
    windows: Query<&Window>,
    assets: Res<Assets<Image>>,
    mut sprite_position: Query<
        (
            &Id,
            &Handle<Image>,
            &Position,
            &Scale,
            &Rotation,
            &mut Transform,
            &mut Sprite,
        ),
        With<MyImage>,
    >,
    layout_query: Query<&GridLayout>,
    mouse_query: Query<&MouseState>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let layout = layout_query.single();
    let mouse = mouse_query.single();
    let window = windows.single();
    let length = sprite_position.iter().count();

    let (get_position, cell_size_layout): (Box<dyn Fn(f32) -> Vec2>, Vec2) = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / length as f32, 0.);
            let offset = Vec2::new(-window.width() / 2. + step.x / 2., 0.);
            let cell_size = Vec2::new(step.x, window.height());
            let get_position = move |index| index * step + offset;
            (Box::new(get_position), cell_size)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., -window.height() / length as f32);
            let offset = Vec2::new(0., window.height() / 2. + step.y / 2.);
            let cell_size = Vec2::new(window.width(), step.y.abs());
            let get_position = move |index| index * step + offset;
            (Box::new(get_position), cell_size)
        }
        GridLayout::Stack => {
            let step = Vec2::new(0., 0.);
            let offset = Vec2::new(0., 0.);
            let cell_size = Vec2::new(window.width(), window.height());
            let get_position = move |index| index * step + offset;
            (Box::new(get_position), cell_size)
        }
        GridLayout::Grid => {
            let grid_width = (length as f32).sqrt().ceil();
            let grid_height = (length as f32 / grid_width).ceil();
            let step = Vec2::new(window.width() / grid_width, -window.height() / grid_height);
            let offset = Vec2::new(
                -window.width() / 2. + step.x / 2.,
                window.height() / 2. + step.y / 2.,
            );
            let cell_size = step.abs();
            let get_position = move |index| {
                let row_index = f32::floor(index / grid_width);
                let col_index = f32::rem_euclid(index, grid_width);
                Vec2::new(col_index, row_index) * step + offset
            };
            (Box::new(get_position), cell_size)
        }
    };

    for (id, image_handle, position, scale, rotation, mut transform, mut sprite) in
        &mut sprite_position
    {
        let Some(image) = assets.get(image_handle) else {
            continue;
        };
        let image_size = image.size();

        transform.translation = get_position(id.0 as f32).extend(transform.translation.z);
        transform.scale = scale.0.extend(1.);
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation.0);
        let delta = Vec2::from_angle(PI / 2. * rotation.0).rotate(position.0 + mouse.delta)
            * Vec2::new(1., -1.)
            / scale.0;
        let image_crop = Rect::from_center_size(image_size / 2., image_size);
        let rotate_cell_size = if rotation.0 % 2. == 0. {
            cell_size_layout
        } else {
            Vec2::new(cell_size_layout.y, cell_size_layout.x)
        };
        let cell_center_area = Rect::from_center_size(
            image_size / 2.,
            (image_size - rotate_cell_size / scale.0).max(Vec2::ONE),
        );
        let cell = Rect::from_center_size(
            bound(image_size / 2. - delta, cell_center_area),
            (rotate_cell_size - 2.) / scale.0,
        );

        sprite.rect = Some(cell.intersect(image_crop));
    }
}

fn on_move_image_title(
    mut move_image_evr: EventReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&Id, &mut Style), With<MyText>>,
    layout_query: Query<&GridLayout>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let layout = layout_query.single();
    let window = windows.single();
    let length = text_query.iter().count();

    let get_position: Box<dyn Fn(f32) -> Vec2> = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / length as f32, 0.);
            Box::new(move |index| index * step)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., window.height() / length as f32);
            Box::new(move |index| index * step)
        }
        GridLayout::Stack => Box::new(move |_| Vec2::ZERO),
        GridLayout::Grid => {
            let grid_width = (length as f32).sqrt().ceil();
            let grid_height = (length as f32 / grid_width).ceil();
            let step = Vec2::new(window.width() / grid_width, window.height() / grid_height);

            let get_position = move |index| {
                let row_index = f32::floor(index / grid_width);
                let col_index = f32::rem_euclid(index, grid_width);
                Vec2::new(col_index, row_index) * step
            };
            Box::new(get_position)
        }
    };

    for (id, mut style) in &mut text_query {
        let pos = get_position(id.0 as f32);
        style.position = UiRect {
            top: Val::Px(pos.y + 2.),
            left: Val::Px(pos.x + 5.),
            ..default()
        };
    }
}

fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&Id, &mut Style, &CalculatedSize), With<MyCursor>>,
    layout_query: Query<&GridLayout>,
) {
    let layout = layout_query.single();
    let window = windows.single();
    let length = cursor_query.iter().count();

    let (get_position, cell_size): (Box<dyn Fn(f32) -> Vec2>, Vec2) = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / length as f32, 0.);
            let cell_size = Vec2::new(step.x, window.height());
            (Box::new(move |index| index * step), cell_size)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., window.height() / length as f32);
            let cell_size = Vec2::new(window.width(), step.y.abs());
            (Box::new(move |index| index * step), cell_size)
        }
        GridLayout::Stack => {
            let cell_size = Vec2::new(window.width(), window.height());
            (Box::new(move |_| Vec2::ZERO), cell_size)
        }
        GridLayout::Grid => {
            let grid_width = (length as f32).sqrt().ceil();
            let grid_height = (length as f32 / grid_width).ceil();
            let step = Vec2::new(window.width() / grid_width, window.height() / grid_height);
            let cell_size = step.abs();
            let get_position = move |index| {
                let row_index = f32::floor(index / grid_width);
                let col_index = f32::rem_euclid(index, grid_width);
                Vec2::new(col_index, row_index) * step
            };
            (Box::new(get_position), cell_size)
        }
    };

    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    for (id, mut style, size) in &mut cursor_query {
        let pos = get_position(id.0 as f32);
        style.position = UiRect {
            top: Val::Px(
                pos.y + cell_size.y
                    - f32::rem_euclid(cursor_position.y, cell_size.y)
                    - size.size.y / 2.,
            ),
            left: Val::Px(
                pos.x + f32::rem_euclid(cursor_position.x, cell_size.x) - size.size.x / 2.,
            ),
            ..default()
        };
    }
}

fn on_resize_system(
    mut resize_evr: EventReader<WindowResized>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
) {
    for _ in resize_evr.iter() {
        move_image_evw.send(MoveImageEvent);
    }
}

fn on_reset_visibility(
    mut reset_evr: EventReader<ResetVisibilityEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    layout_query: Query<&mut GridLayout>,
) {
    for _ in reset_evr.iter() {
        let layout = layout_query.single();
        for (i, mut visibility) in &mut visibility_query {
            *visibility = match *layout {
                GridLayout::Stack => {
                    if i.0 == 0 {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    }
                }
                _ => Visibility::Visible,
            }
        }
    }
}

fn change_layout(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
    mut layout_query: Query<&mut GridLayout>,
) {
    if keys.just_pressed(KeyCode::L) {
        let mut layout = layout_query.single_mut();
        *layout = match *layout {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Vertical,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Grid,
        };
        reset_vix_evw.send(ResetVisibilityEvent);
        move_image_evw.send(MoveImageEvent);
    }
}

fn change_layout_on_click(
    buttons: Res<Input<MouseButton>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
    mut layout_query: Query<&mut GridLayout>,
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

        let mut layout = layout_query.single_mut();
        *layout = match *layout {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Grid,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Vertical,
        };
        reset_vix_evw.send(ResetVisibilityEvent);
        move_image_evw.send(MoveImageEvent);
    }
}

fn change_top_image(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    layout_query: Query<&GridLayout>,
) {
    let modifier_pressed = keys.pressed(KeyCode::LShift) || keys.pressed(KeyCode::RShift);
    let index_on_top = if modifier_pressed && keys.just_pressed(KeyCode::Key1) {
        1
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key2) {
        2
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key3) {
        3
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key4) {
        4
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key5) {
        5
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key6) {
        6
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key7) {
        7
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key8) {
        8
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key9) {
        9
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key0) {
        10
    } else {
        return;
    };

    let layout = layout_query.single();
    for (i, mut visibility) in &mut visibility_query {
        *visibility = match layout {
            GridLayout::Stack => {
                if i.0 == index_on_top - 1 {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                }
            }
            _ => Visibility::Visible,
        };
    }

    move_image_evw.send(MoveImageEvent);
}

fn change_rotation_image(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut rotation_query: Query<&mut Rotation, With<MyImage>>,
) {
    if keys.just_pressed(KeyCode::R) {
        for mut rotation in &mut rotation_query {
            rotation.0 += 1.;
        }
    };
    move_image_evw.send(MoveImageEvent);
}

fn change_zoom(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut query: Query<(&mut Scale, &mut Position)>,
) {
    let modifier_pressed = keys.pressed(KeyCode::LControl) || keys.pressed(KeyCode::RControl);
    let scale_factor = if modifier_pressed && keys.just_pressed(KeyCode::Key1) {
        0.
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key2) {
        1.
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key3) {
        3.
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key4) {
        4.
    } else if modifier_pressed && keys.just_pressed(KeyCode::Key5) {
        5.
    } else {
        return;
    };

    for (mut scale, mut position) in &mut query {
        let zoom_factor = (2_f32).powf(scale_factor);
        position.0 *= zoom_factor / scale.0.x;
        scale.0.x = zoom_factor;
        scale.0.y = zoom_factor;
    }

    move_image_evw.send(MoveImageEvent);
}

fn toggle_help(keys: Res<Input<KeyCode>>, mut query: Query<&mut Visibility, With<MyHelp>>) {
    if keys.just_pressed(KeyCode::H) {
        let mut visibility = query.single_mut();
        *visibility = match *visibility {
            Visibility::Visible => Visibility::Hidden,
            Visibility::Hidden => Visibility::Visible,
            Visibility::Inherited => Visibility::Inherited,
        };
    }
}

fn toggle_cursor(
    keys: Res<Input<KeyCode>>,
    mut windows: Query<&mut Window>,
    mut commands: Commands,
    cursor_query: Query<Entity, With<MyCursor>>,
    image_query: Query<&Id, With<MyImage>>,
    font_query: Query<&FontHandle>,
) {
    if keys.just_pressed(KeyCode::C) {
        if cursor_query.iter().count() == 0 {
            let mut window = windows.single_mut();
            window.cursor.icon = CursorIcon::Crosshair;
            let font = font_query.single();
            for id in &image_query {
                commands.spawn((
                    TextBundle::from_section(
                        "+",
                        TextStyle {
                            font: font.0.clone(),
                            font_size: 28.0,
                            color: Color::ORANGE_RED,
                        },
                    )
                    .with_text_alignment(TextAlignment::Center)
                    .with_style(Style {
                        position_type: PositionType::Absolute,
                        ..default()
                    }),
                    Id(id.0),
                    MyCursor,
                ));
            }
        } else {
            let mut window = windows.single_mut();
            window.cursor.icon = CursorIcon::Default;
            for entity in &cursor_query {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn scroll_events(
    mut scroll_evr: EventReader<MouseWheel>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut query: Query<(&mut Scale, &mut Position)>,
) {
    use bevy::input::mouse::MouseScrollUnit;
    for ev in scroll_evr.iter() {
        let scroll = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y,
        };

        for (mut scale, mut position) in &mut query {
            let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
            scale.0.x *= zoom_factor;
            scale.0.y *= zoom_factor;
            position.0 *= zoom_factor;
        }
        move_image_evw.send(MoveImageEvent);
    }
}

fn mouse_button_input(
    buttons: Res<Input<MouseButton>>,
    windows: Query<&Window>,
    mut mouse_query: Query<&mut MouseState>,
    mut position_query: Query<&mut Position>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let window = windows.single();
        if let Some(cursor_position) = window.cursor_position() {
            let mut mouse_state = mouse_query.single_mut();
            mouse_state.pressed = true;
            mouse_state.origin = cursor_position;
            mouse_state.delta = Vec2::ZERO;
        }
    }
    if buttons.just_released(MouseButton::Left) {
        let window = windows.single();
        if let Some(cursor_position) = window.cursor_position() {
            let mut mouse_state = mouse_query.single_mut();
            mouse_state.pressed = false;
            for mut position in &mut position_query {
                position.0 += cursor_position - mouse_state.origin;
            }
            mouse_state.origin = cursor_position;
            mouse_state.delta = Vec2::ZERO;
        }
    }
}

fn cursor_events(
    mut cursor_evr: EventReader<CursorMoved>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut mouse_query: Query<&mut MouseState>,
) {
    for ev in cursor_evr.iter() {
        let mut mouse_state = mouse_query.single_mut();
        if mouse_state.pressed {
            mouse_state.delta = ev.position - mouse_state.origin;
            move_image_evw.send(MoveImageEvent);
        }
    }
}

fn file_drop(
    mut dnd_evr: EventReader<FileDragAndDrop>,
    mut load_image_evw: EventWriter<LoadNewImageEvent>,
) {
    let mut images_filename = Vec::new();

    for ev in dnd_evr.iter() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = ev {
            let Some(image_absolute) = path_buf.as_path().to_str() else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            images_filename.push(String::from(image_absolute));
        }
    }
    let count = images_filename.iter().count();
    for (index, filename) in images_filename.into_iter().enumerate() {
        load_image_evw.send(LoadNewImageEvent {
            path: filename,
            index: index as i8,
            count: count as i8,
        });
    }
}

fn bound(vec: Vec2, rect: Rect) -> Vec2 {
    Vec2::new(
        vec.x.clamp(rect.min.x, rect.max.x),
        vec.y.clamp(rect.min.y, rect.max.y),
    )
}

fn check_all_images_exist(images: &Vec<String>) -> Result<Vec<String>> {
    let mut images_absolute = Vec::new();
    for image_filename in images {
        let input_path = Path::new(&image_filename);
        if !input_path.exists() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Image not found: {}", image_filename),
            )));
        }
        if !input_path.is_file() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Provided Path is not a file: {}", image_filename),
            )));
        }
        let resolved_path = canonicalize(input_path)?;
        let Some(image_absolute) = resolved_path.as_path().to_str() else {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Can't resolve given path: {}", image_filename),
            )));
        };
        images_absolute.push(String::from(image_absolute));
    }
    Ok(images_absolute)
}

fn get_short_name(path: &String) -> Option<&str> {
    Path::new(path).file_name()?.to_str()
}
