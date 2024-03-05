// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

use std::fs::canonicalize;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, Instant};

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
    Double-click: Switch between layout Grid-Stack or Horizontal-Vertical
    R: Rotate images
    Shift + 1, 2, 3, ...: Move Image on top
    Ctrl/Cmd + 1, 2, 3, 4, 5: Zoom by 1, 2, 4, 8, 16
    Ctrl/Cmd + Shift + 1, 2, 3, 4, 5: Zoom by 1/2, 1/4, 1/8, 1/16, 1/32
    Z + Right/Left clic: zoom in/out the hovered image only
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
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(InitialImagesFilename(images_filename))
        .add_systems(Startup, setup)
        .add_event::<LoadNewImageEvent>()
        .add_event::<NewImageLoadedEvent>()
        .add_event::<MoveImageEvent>()
        .add_event::<ResetVisibilityEvent>()
        .add_systems(Update, change_layout)
        .add_systems(Update, change_layout_on_click)
        .add_systems(Update, change_global_zoom)
        .add_systems(Update, change_zoom_individually)
        .add_systems(Update, scroll_events)
        .add_systems(Update, mouse_button_input)
        .add_systems(Update, cursor_move)
        .add_systems(Update, file_drop)
        .add_systems(Update, change_top_image)
        .add_systems(Update, change_rotation_image)
        .add_systems(Update, on_reset_visibility)
        .add_systems(Update, on_resize_system)
        .add_systems(Update, on_image_loaded)
        .add_systems(Update, on_move_cursor)
        .add_systems(Update, on_move_image)
        .add_systems(Update, on_move_image_title)
        .add_systems(Update, on_load_image)
        .add_systems(Update, toggle_help)
        .add_systems(Update, toggle_cursor)
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
struct Id(usize);

#[derive(Component)]
struct GlobalScale(f32);

#[derive(Component)]
struct Scale(f32);

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

#[derive(Event)]
struct MoveImageEvent;

#[derive(Event)]
struct ResetVisibilityEvent;

#[derive(Event)]
struct NewImageLoadedEvent {
    handle: Handle<Image>,
    path: String,
    index: usize,
    count: usize,
}

#[derive(Component)]
struct TotalImageLoaded(usize);

#[derive(Component)]
struct FontHandle(Handle<Font>);

#[derive(Event)]
struct LoadNewImageEvent {
    path: String,
    index: usize,
    count: usize,
}

fn setup(
    mut commands: Commands,
    images_filename: ResMut<InitialImagesFilename>,
    mut load_image_evw: EventWriter<LoadNewImageEvent>,
    mut fonts: ResMut<Assets<Font>>,
) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(GridLayout::Grid);
    commands.spawn(GlobalScale(1. / 8.));
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
            top: Val::Px(22.),
            left: Val::Px(5.),
            ..default()
        }),
        MyHelp,
    ));

    let count = images_filename.0.len();
    for (index, image) in images_filename.0.iter().enumerate() {
        load_image_evw.send(LoadNewImageEvent {
            path: image.clone(),
            index: index,
            count: count,
        });
    }
}

fn on_load_image(
    mut load_evr: EventReader<LoadNewImageEvent>,
    mut loaded_evw: EventWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
) {
    for ev in load_evr.read() {
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

        // Remove the memory limit on image size we can read
        reader.no_limits();

        let Some(image) = reader.decode().ok() else {
            println!("Failed to decode image");
            continue;
        };

        match image.color() {
            ColorType::Rgb8 | ColorType::Rgba8 | ColorType::L8 | ColorType::La8 => {
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
    for ev in load_image_evr.read() {
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
            Scale(1.),
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
    global_scale_query: Query<&GlobalScale>,
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
    let num_images = sprite_position.iter().count();
    let global_scale = global_scale_query.single();

    for (id, image_handle, position, scale, rotation, mut transform, mut sprite) in
        &mut sprite_position
    {
        let Some(image) = assets.get(image_handle) else {
            continue;
        };
        let image_size = image.size().as_vec2();

        let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, layout, window);
        transform.translation =
            (Vec2::new(-window.width() / 2., -window.height() / 2.) + cell_offset + cell_size / 2.)
                .extend(transform.translation.z) * Vec3::new(1., -1., 1.);
        transform.scale = Vec2::splat(scale.0 * global_scale.0).extend(1.);
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation.0);

        let delta =
            Vec2::from_angle(PI / 2. * rotation.0).rotate(position.0 + mouse.delta / (scale.0 * global_scale.0));
        let image_crop = Rect::from_center_size(image_size / 2., image_size);
        let rotated_cell_size = if rotation.0 % 2. == 0. {
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

    let num_images = text_query.iter().count();
    let layout = layout_query.single();
    let window = windows.single();

    for (id, mut style) in &mut text_query {
        let (cell_offset, _) = get_cell_rect(id.0, num_images, layout, window);
        style.top = Val::Px(cell_offset.y + 2.);
        style.left = Val::Px(cell_offset.x + 5.);
    }
}

fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&Id, &mut Transform), With<MyCursor>>,
    layout_query: Query<&GridLayout>,
) {
    let num_images = cursor_query.iter().count();
    let layout = layout_query.single();
    let window = windows.single();

    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    for (id, mut transform) in &mut cursor_query {
        let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, layout, window);
        let new_y = cell_offset.y + f32::rem_euclid(cursor_position.y, cell_size.y);
        let new_x = cell_offset.x + f32::rem_euclid(cursor_position.x, cell_size.x);
        transform.translation = Vec3::new(
            -window.width() / 2. + new_x,
            window.height() / 2. - new_y,
            transform.translation.z,
        );
    }
}

fn on_resize_system(
    mut resize_evr: EventReader<WindowResized>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
) {
    for _ in resize_evr.read() {
        move_image_evw.send(MoveImageEvent);
    }
}

fn on_reset_visibility(
    mut reset_evr: EventReader<ResetVisibilityEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    layout_query: Query<&mut GridLayout>,
) {
    for _ in reset_evr.read() {
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
    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl_pressed {
        return;
    }
    let shift_pressed = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let index_on_top = if shift_pressed && keys.just_pressed(KeyCode::Key1) {
        1
    } else if shift_pressed && keys.just_pressed(KeyCode::Key2) {
        2
    } else if shift_pressed && keys.just_pressed(KeyCode::Key3) {
        3
    } else if shift_pressed && keys.just_pressed(KeyCode::Key4) {
        4
    } else if shift_pressed && keys.just_pressed(KeyCode::Key5) {
        5
    } else if shift_pressed && keys.just_pressed(KeyCode::Key6) {
        6
    } else if shift_pressed && keys.just_pressed(KeyCode::Key7) {
        7
    } else if shift_pressed && keys.just_pressed(KeyCode::Key8) {
        8
    } else if shift_pressed && keys.just_pressed(KeyCode::Key9) {
        9
    } else if shift_pressed && keys.just_pressed(KeyCode::Key0) {
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

fn change_global_zoom(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut global_scale: Query<&mut GlobalScale>,
) {
    let mut global_scale = global_scale.single_mut();
    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift_pressed = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let scale_factor = if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Key1) {
        -1.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Key2) {
        -2.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Key3) {
        -3.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Key4) {
        -4.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Key5) {
        -5.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Key1) {
        0.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Key2) {
        1.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Key3) {
        3.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Key4) {
        4.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Key5) {
        5.
    } else {
        return;
    };

    let zoom_factor = (2_f32).powf(scale_factor);
    global_scale.0 = zoom_factor;

    move_image_evw.send(MoveImageEvent);
}

fn change_zoom_individually(
    windows: Query<&Window>,
    keys: Res<Input<KeyCode>>,
    buttons: Res<Input<MouseButton>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    layout_query: Query<&GridLayout>,
    mut sprite_query: Query<(&Id, &mut Scale), With<MyImage>>,
) {
    if keys.pressed(KeyCode::Z) {
        if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
            return;
        }

        let num_images = sprite_query.iter().count();
        let layout = layout_query.single();
        let window = windows.single();

        let Some(cursor_position) = window.cursor_position() else {
            return;
        };
        let scale_factor = if buttons.just_pressed(MouseButton::Left) {
            2.0f32
        } else {
            0.5f32
        };

        for (id, mut scale) in &mut sprite_query {
            let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, layout, window);

            if cursor_position.x > cell_offset.x
                && cursor_position.x < cell_offset.x + cell_size.x
                && cursor_position.y > cell_offset.y
                && cursor_position.y < cell_offset.y + cell_size.y
            {
                scale.0 *= scale_factor;
                break;
            }
        }

        move_image_evw.send(MoveImageEvent);
    }
}

fn get_cell_rect(
    index: usize,
    num_images: usize,
    layout: &GridLayout,
    window: &Window,
) -> (Vec2, Vec2) {
    let (cell_tl, cell_size): (Vec2, Vec2) = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / num_images as f32, 0.);
            let cell_size = Vec2::new(step.x, window.height());
            (index as f32 * step, cell_size)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., window.height() / num_images as f32);
            let cell_size = Vec2::new(window.width(), step.y.abs());
            (index as f32 * step, cell_size)
        }
        GridLayout::Stack => {
            let cell_size = Vec2::new(window.width(), window.height());
            (Vec2::ZERO, cell_size)
        }
        GridLayout::Grid => {
            let grid_width = (num_images as f32).sqrt().ceil();
            let grid_height = (num_images as f32 / grid_width).ceil();
            let step = Vec2::new(window.width() / grid_width, window.height() / grid_height);
            let cell_size = step.abs();
            let row_index = f32::floor(index as f32 / grid_width);
            let col_index = f32::rem_euclid(index as f32, grid_width);
            (Vec2::new(col_index, row_index) * step, cell_size)
        }
    };
    (cell_tl, cell_size)
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
) {
    if keys.just_pressed(KeyCode::C) {
        if cursor_query.iter().count() == 0 {
            let mut window = windows.single_mut();
            window.cursor.visible = false;
            for id in &image_query {
                commands
                    .spawn((
                        SpatialBundle {
                            transform: Transform::from_translation(Vec3::new(0., 0., 1.)),
                            ..default()
                        },
                        Id(id.0),
                        MyCursor,
                    ))
                    .with_children(|parent| {
                        let cursor_color = Color::rgb(0.75, 0., 0.);
                        let bar_size = 15.;
                        let cursor_size = Some(Vec2::new(bar_size, 4.0));
                        parent.spawn(SpriteBundle {
                            sprite: Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            transform: Transform::from_rotation(Quat::from_rotation_z(-TAU / 4.))
                                .with_translation(Vec3::new(bar_size, 0., 1.)),
                            ..default()
                        });
                        parent.spawn(SpriteBundle {
                            sprite: Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            transform: Transform::from_rotation(Quat::from_rotation_z(-TAU / 4.))
                                .with_translation(Vec3::new(-bar_size, 0., 1.)),
                            ..default()
                        });
                        parent.spawn(SpriteBundle {
                            sprite: Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            transform: Transform::from_translation(Vec3::new(0., bar_size, 1.)),
                            ..default()
                        });
                        parent.spawn(SpriteBundle {
                            sprite: Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            transform: Transform::from_translation(Vec3::new(0., -bar_size, 1.)),
                            ..default()
                        });
                    });
            }
        } else {
            let mut window = windows.single_mut();
            window.cursor.visible = true;
            for entity in &cursor_query {
                commands.entity(entity).despawn_recursive();
            }
        }
    }
}

fn scroll_events(
    mut scroll_evr: EventReader<MouseWheel>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut global_scale_query: Query<&mut GlobalScale>,
) {
    let mut global_scale = global_scale_query.single_mut();
    use bevy::input::mouse::MouseScrollUnit;
    for ev in scroll_evr.read() {
        let scroll = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y,
        };

        let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
        global_scale.0 *= zoom_factor;

        move_image_evw.send(MoveImageEvent);
    }
}

fn mouse_button_input(
    buttons: Res<Input<MouseButton>>,
    windows: Query<&Window>,
    mut mouse_query: Query<&mut MouseState>,
    global_scale_query: Query<&GlobalScale>,
    mut image_query: Query<(&mut Position, &Scale), With<MyImage>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let window = windows.single();
        let Some(cursor_position) = window.cursor_position() else {
            return;
        };
        let mut mouse = mouse_query.single_mut();
        mouse.pressed = true;
        mouse.origin = cursor_position;
        mouse.delta = Vec2::ZERO;
    }
    if buttons.just_released(MouseButton::Left) {
        let mut mouse = mouse_query.single_mut();
        let global_scale = global_scale_query.single();
        mouse.pressed = false;

        let window = windows.single();

        let Some(cursor_position) = window.cursor_position() else {
            // Cursor is outside of the windows
            for (mut position, scale) in &mut image_query {
                position.0 += mouse.delta / (scale.0 * global_scale.0);
                let delta = mouse.delta;
                mouse.origin += delta / (scale.0 * global_scale.0);
            }
            mouse.delta = Vec2::ZERO;
            return;
        };

        for (mut position, scale) in &mut image_query {
            position.0 += (cursor_position - mouse.origin) / (scale.0 * global_scale.0);
        }
        mouse.origin = cursor_position;
        mouse.delta = Vec2::ZERO;
    }
}

fn cursor_move(
    mut cursor_evr: EventReader<CursorMoved>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut mouse_query: Query<&mut MouseState>,
) {
    for ev in cursor_evr.read() {
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

    for ev in dnd_evr.read() {
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
            index: index,
            count: count,
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
