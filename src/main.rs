// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

use std::f32::consts::{PI, TAU};
use std::fs::canonicalize;
use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;
use std::path::Path;
use std::time::{Duration, Instant};

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, Window, WindowResized};
use clap::Parser;
use home;
use image::{ColorType, DynamicImage, ImageFormat, SubImage};
use serde::Deserialize;

#[doc(hidden)]
type Result<T> = ::std::result::Result<T, Box<dyn ::std::error::Error>>;

#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    // Images to show
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
    P: Save image to disk with the displayed crop (prefixed by cr_)
    H: Toggle this help

    Drag and Drop image from files explorer.
";

#[derive(Deserialize, Debug)]
struct ConfigShortcut {
    save_crop_image: KeyCode,
    local_zoom_modifier: KeyCode,
    switch_cursor: KeyCode,
    switch_layout: KeyCode,
    rotate_images: KeyCode,
}

#[derive(Deserialize, Debug)]
struct ConfigText {
    font_size: f32,
    font_color: Color,
}

#[derive(Deserialize, Debug)]
struct ConfigHDR {
    enabled: bool,
}

#[derive(Deserialize, Debug)]
struct ConfigMisc {
    enable_zoom_on_scroll: bool,
}

#[derive(Deserialize, Debug, Resource)]
struct Config {
    text: ConfigText,
    shortcut: ConfigShortcut,
    hdr: ConfigHDR,
    misc: ConfigMisc,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let images_filename = check_all_images_exist(&args.images)?;

    let user_config_data = 'block: {
        let Some(home_directory) = home::home_dir() else {
            println!("User directory not found");
            break 'block None;
        };
        println!("User Directory Found: {}", home_directory.display());
        let config_filename = ".image_viewer";
        println!("{}", home_directory.join(config_filename).display());

        let Some(config_str) = std::fs::read_to_string(home_directory.join(config_filename)).ok()
        else {
            println!(
                "Config File not found: {}",
                home_directory.join(config_filename).display()
            );
            break 'block None;
        };

        toml::from_str(&config_str)?
    };

    let config_data = match user_config_data {
        Some(data) => data,
        None => {
            let config_str = include_str!("../assets/default/config.toml");
            let Some(config): Option<Config> = toml::from_str(config_str).ok() else {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Invalid Default config : You messed up somewhere it should not happened",
                )));
            };
            config
        }
    };

    println!("Config: {:?}", config_data);

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Image Viewer 3000".to_string(),
                        resolution: [600., 350.].into(),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .insert_resource(InitialImagesFilename(images_filename))
        .insert_resource(config_data)
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
        .add_systems(Update, save_cropped)
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
struct ImagePath(String);

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
    config: Res<Config>,
    mut load_image_evw: EventWriter<LoadNewImageEvent>,
    mut fonts: ResMut<Assets<Font>>,
) {
    commands.spawn(Camera2dBundle {
        camera: Camera {
            hdr: config.hdr.enabled,
            ..default()
        },
        // tonemapping: Tonemapping::TonyMcMapface,
        ..default()
    });

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
                let new_image = Image::from_dynamic(image, true);
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            ColorType::L16 => {
                let image_rgb16 = DynamicImage::ImageRgb16(image.into_rgb16());
                let new_image = Image::from_dynamic(image_rgb16, true);
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            _ => {
                println!(
                    "Unsupported image type : image.color(): {:?}",
                    image.color()
                )
            }
        }
    }
}

fn on_image_loaded(
    config: Res<Config>,
    mut load_image_evr: EventReader<NewImageLoadedEvent>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vis_evw: EventWriter<ResetVisibilityEvent>,
    mut commands: Commands,
    images: Query<Entity, With<Id>>,
    mut count_query: Query<&mut TotalImageLoaded>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
    layout_query: Query<&GridLayout>,
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
        let layout = layout_query.single();
        let visibility = match layout {
            GridLayout::Stack => {
                if already_loaded.0 != 0 {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                }
            }
            _ => Visibility::Visible,
        };

        commands.spawn((
            SpriteBundle {
                texture: ev.handle.clone(),
                visibility,
                ..default()
            },
            Id(ev.index),
            Scale(1.),
            Position(Vec2::ZERO),
            Rotation(0.),
            ImagePath(ev.path.clone()),
            MyImage,
        ));

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            TextBundle {
                text: Text::from_section(
                    short_path,
                    TextStyle {
                        font: font.0.clone(),
                        font_size: config.text.font_size,
                        color: config.text.font_color,
                    },
                ),
                visibility,
                style: Style {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                ..default()
            }
            .with_text_alignment(TextAlignment::Left),
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
    mut title_query: Query<&mut Style, With<MyText>>,
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
                .extend(transform.translation.z)
                * Vec3::new(1., -1., 1.);
        transform.scale = Vec2::splat(scale.0 * global_scale.0).extend(1.);
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation.0);

        let delta = Vec2::from_angle(PI / 2. * rotation.0)
            .rotate(position.0 + mouse.delta / (scale.0 * global_scale.0));
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

    let (_, cell_size) = get_cell_rect(0, num_images, layout, window);
    for mut style in &mut title_query {
        style.width = Val::Px(cell_size.x);
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
    config: Res<Config>,
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
    mut layout_query: Query<&mut GridLayout>,
) {
    if keys.just_pressed(config.shortcut.switch_layout) {
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
    config: Res<Config>,
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut rotation_query: Query<&mut Rotation, With<MyImage>>,
) {
    if keys.just_pressed(config.shortcut.rotate_images) {
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
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<Input<KeyCode>>,
    buttons: Res<Input<MouseButton>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    layout_query: Query<&GridLayout>,
    mut sprite_query: Query<(&Id, &mut Scale, &mut Position), With<MyImage>>,
) {
    if keys.pressed(config.shortcut.local_zoom_modifier) {
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

        let position_normalized = 'outer: {
            for (id, mut scale, position) in &mut sprite_query {
                let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, layout, window);

                if cursor_position.x > cell_offset.x
                    && cursor_position.x < cell_offset.x + cell_size.x
                    && cursor_position.y > cell_offset.y
                    && cursor_position.y < cell_offset.y + cell_size.y
                {
                    scale.0 *= scale_factor;
                    break 'outer Some(position.0 * scale.0);
                }
            }
            None
        };

        if let Some(pos) = position_normalized {
            // Reset position for other images to match the one we zoom
            for (_id, scale, mut position) in &mut sprite_query {
                position.0 = pos / scale.0;
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
    config: Res<Config>,
    keys: Res<Input<KeyCode>>,
    mut windows: Query<&mut Window>,
    mut commands: Commands,
    cursor_query: Query<Entity, With<MyCursor>>,
    image_query: Query<&Id, With<MyImage>>,
) {
    if keys.just_pressed(config.shortcut.switch_cursor) {
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

fn save_cropped(
    config: Res<Config>,
    keys: Res<Input<KeyCode>>,
    image_query: Query<(&ImagePath, &Sprite), With<MyImage>>,
) {
    if keys.just_pressed(config.shortcut.save_crop_image) {
        for (path, sprite) in &image_query {
            // Get Input image
            let input_path = Path::new(&path.0);
            let Some(f_in) = File::open(&input_path).ok() else {
                println!("Failed to open file {}", path.0);
                continue;
            };
            let buf_in = BufReader::new(f_in);
            let Some(format) = ImageFormat::from_path(&input_path).ok() else {
                println!("Failed to deduce image format from path : {}", path.0);
                continue;
            };

            let mut reader = image::io::Reader::with_format(buf_in, format);

            // Remove the memory limit on image size we can read
            reader.no_limits();

            let Some(image) = reader.decode().ok() else {
                println!("Failed to decode image");
                continue;
            };

            // Get Output buffer
            let Some(parent) = input_path.parent() else {
                continue;
            };
            let Some(filename) = input_path.file_name() else {
                continue;
            };
            let Some(filename_as_str) = filename.to_str() else {
                continue;
            };
            let output_path = parent.join(String::from("cr_") + filename_as_str);

            let Some(f_out) = File::create(&output_path).ok() else {
                println!("Failed to create file {}", &output_path.display());
                continue;
            };
            let mut buf_out = BufWriter::new(f_out);

            // Get Roi from sprite
            let Some(rect) = sprite.rect else {
                println!("Failed to get ROI of the texture");
                continue;
            };

            let size = rect.max - rect.min;
            let image_view = SubImage::new(
                &image,
                rect.min.x as u32,
                rect.min.y as u32,
                size.x as u32,
                size.y as u32,
            );

            let subimage = image_view.to_image();

            // Save to disk
            let Some(_) = subimage.write_to(&mut buf_out, ImageFormat::Jpeg).ok() else {
                println!("Failed to write data to file {}", &output_path.display());
                continue;
            };
        }
    }
}

fn scroll_events(
    config: Res<Config>,
    mut scroll_evr: EventReader<MouseWheel>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut global_scale_query: Query<&mut GlobalScale>,
) {
    if config.misc.enable_zoom_on_scroll {
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
