// #![allow(unused_variables)]

use std::fs::canonicalize;
use std::path::Path;

use bevy::diagnostic::LogDiagnosticsPlugin;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowDescriptor, WindowResized};
use clap::Parser;
use image::{ColorType, DynamicImage};

#[doc(hidden)]
type Result<T> = ::std::result::Result<T, Box<dyn ::std::error::Error>>;

#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    /// Images to show
    images: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let images_filename = check_all_images_exist(&args.images)?;

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    window: WindowDescriptor {
                        title: "Image Viewer 3000".to_string(),
                        width: 500.,
                        height: 300.,
                        present_mode: PresentMode::AutoVsync,
                        // always_on_top: true,
                        ..default()
                    },
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugin(LogDiagnosticsPlugin::default())
        .insert_resource(InitialImagesFilename(images_filename))
        .add_startup_system(setup)
        .add_event::<LoadNewImageEvent>()
        .add_event::<NewImageLoadedEvent>()
        .add_event::<MoveImageEvent>()
        .add_system(change_layout)
        .add_system(scroll_events)
        .add_system(mouse_button_input)
        .add_system(cursor_events)
        .add_system(file_drop)
        .add_system(change_top_image)
        .add_system(on_resize_system)
        .add_system(on_image_loaded)
        .add_system(on_move_image)
        .add_system(on_move_image_title)
        .add_system(on_load_image)
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
}

#[derive(Component)]
struct Id(i8);

#[derive(Component)]
struct Scale(Vec2);

#[derive(Component)]
struct Position(Vec2);

#[derive(Component)]
struct MyImage;

#[derive(Component)]
struct MyText;

#[derive(Component)]
struct MouseState {
    origin: Vec2,
    delta: Vec2,
    pressed: bool,
}

struct MoveImageEvent;

struct NewImageLoadedEvent {
    handle: Handle<Image>,
    path: String,
    index: i8,
    count: i8,
}

#[derive(Component)]
struct TotalImageLoaded(i8);

struct LoadNewImageEvent {
    path: String,
    index: i8,
    count: i8,
}

fn setup(
    mut commands: Commands,
    images_filename: ResMut<InitialImagesFilename>,
    mut load_image_evw: EventWriter<LoadNewImageEvent>,
) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(GridLayout::Horizontal);
    commands.spawn(TotalImageLoaded(0));
    commands.spawn(MouseState {
        origin: Vec2::ZERO,
        delta: Vec2::ZERO,
        pressed: false,
    });

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
        let Ok(image) = image::open(&ev.path) else {
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
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    images: Query<Entity, With<Id>>,
    mut count_query: Query<&mut TotalImageLoaded>,
) {
    for ev in load_image_evr.iter() {
        let mut already_loaded = count_query.single_mut();

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
            Scale(Vec2::ONE),
            Position(Vec2::ZERO),
            MyImage,
        ));

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            TextBundle::from_section(
                short_path,
                TextStyle {
                    font: asset_server.load("fonts/IBMPlexMono-Regular.otf"),
                    font_size: 14.0,
                    color: Color::WHITE,
                },
            )
            .with_text_alignment(TextAlignment::TOP_LEFT)
            .with_style(Style {
                position_type: PositionType::Absolute,
                ..default()
            }),
            Id(ev.index),
            MyText,
        ));

        move_image_evw.send(MoveImageEvent);
    }
}

fn on_move_image(
    move_image_evr: EventReader<MoveImageEvent>,
    windows: Res<Windows>,
    assets: Res<Assets<Image>>,
    mut sprite_position: Query<
        (
            &Id,
            &Handle<Image>,
            &Position,
            &Scale,
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
    let window = windows.primary();
    let length = sprite_position.iter().count();

    let (step_layout, offset_layout, cell_size_layout) = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / length as f32, 0.);
            let offset = Vec2::new(-window.width() / 2. + step.x / 2., 0.);
            let cell_size = Vec2::new(step.x, window.height());
            (step, offset, cell_size)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., window.height() / length as f32);
            let offset = Vec2::new(0., -window.height() / 2. + step.y / 2.);
            let cell_size = Vec2::new(window.width(), step.y);
            (step, offset, cell_size)
        }
        GridLayout::Stack => {
            let step = Vec2::new(0., 0.);
            let offset = Vec2::new(0., 0.);
            let cell_size = Vec2::new(window.width(), window.height());
            (step, offset, cell_size)
        }
    };

    for (id, image_handle, position, scale, mut transform, mut sprite) in &mut sprite_position {
        let Some(image) = assets.get(image_handle) else {
            continue;
        };
        let image_size = image.size();

        transform.translation.x = id.0 as f32 * step_layout.x + offset_layout.x;
        transform.translation.y = id.0 as f32 * step_layout.y + offset_layout.y;
        transform.scale.x = scale.0.x;
        transform.scale.y = scale.0.y;

        let delta = (position.0 + mouse.delta) * Vec2::new(1., -1.) / scale.0;
        let image_crop = Rect::from_center_size(image_size / 2., image_size);
        let cell_center_area = Rect::from_center_size(
            image_size / 2.,
            (image_size - cell_size_layout / scale.0).max(Vec2::ONE),
        );
        let cell = Rect::from_center_size(
            bound(image_size / 2. - delta, cell_center_area),
            cell_size_layout / scale.0,
        );

        sprite.rect = Some(cell.intersect(image_crop));
    }
}

fn on_move_image_title(
    move_image_evr: EventReader<MoveImageEvent>,
    windows: Res<Windows>,
    mut text_query: Query<(&Id, &mut Style), With<MyText>>,
    layout_query: Query<&GridLayout>
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let layout = layout_query.single();
    let window = windows.primary();
    let length = text_query.iter().count();

    let (step_layout, offset_layout) = match layout {
        GridLayout::Horizontal => {
            let step = Vec2::new(window.width() / length as f32, 0.);
            let offset = Vec2::new(0., 0.);
            (step, offset)
        }
        GridLayout::Vertical => {
            let step = Vec2::new(0., window.height() / length as f32);
            let offset = Vec2::new(0., 0.);
            (step, offset)
        }
        GridLayout::Stack => {
            let step = Vec2::new(0., 0.);
            let offset = Vec2::new(0., 0.);
            (step, offset)
        }
    };

    for (id, mut style) in &mut text_query {
        let pos = id.0 as f32 * step_layout + offset_layout;
        style.position = UiRect {
            top: Val::Px(pos.y + 2.),
            left: Val::Px(pos.x + 5.),
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

fn change_layout(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    mut layout_query: Query<&mut GridLayout>,
) {
    if keys.just_pressed(KeyCode::L) {
        let mut layout = layout_query.single_mut();
        *layout = match *layout {
            GridLayout::Stack => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Vertical,
            GridLayout::Vertical => GridLayout::Stack,
        };
        for (i, mut visibility) in &mut visibility_query {
            visibility.is_visible = match *layout {
                GridLayout::Stack => i.0 == 0,
                _ => true,
            }
        }
        move_image_evw.send(MoveImageEvent);
    }
}
fn change_top_image(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    layout_query: Query<&GridLayout>,
) {
    let index_on_top = if keys.just_pressed(KeyCode::Key1) {
        1
    } else if keys.just_pressed(KeyCode::Key2) {
        2
    } else if keys.just_pressed(KeyCode::Key3) {
        3
    } else if keys.just_pressed(KeyCode::Key4) {
        4
    } else if keys.just_pressed(KeyCode::Key5) {
        5
    } else if keys.just_pressed(KeyCode::Key6) {
        6
    } else if keys.just_pressed(KeyCode::Key7) {
        7
    } else if keys.just_pressed(KeyCode::Key8) {
        8
    } else if keys.just_pressed(KeyCode::Key9) {
        9
    } else if keys.just_pressed(KeyCode::Key0) {
        10
    } else {
        return;
    };

    let layout = layout_query.single();
    for (i, mut visibility) in &mut visibility_query {
        visibility.is_visible = match layout {
            GridLayout::Stack => i.0 == index_on_top - 1,
            _ => true,
        }
    }

    move_image_evw.send(MoveImageEvent);
}

fn scroll_events(
    mut scroll_evr: EventReader<MouseWheel>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut query: Query<&mut Scale>,
) {
    use bevy::input::mouse::MouseScrollUnit;
    for ev in scroll_evr.iter() {
        let scroll = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y,
        };

        for mut scale in &mut query {
            let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
            scale.0.x *= zoom_factor;
            scale.0.y *= zoom_factor;
        }
        move_image_evw.send(MoveImageEvent);
    }
}

fn mouse_button_input(
    buttons: Res<Input<MouseButton>>,
    windows: Res<Windows>,
    mut mouse_query: Query<&mut MouseState>,
    mut position_query: Query<&mut Position>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let window = windows.get_primary().unwrap();
        if let Some(cursor_position) = window.cursor_position() {
            let mut mouse_state = mouse_query.single_mut();
            mouse_state.pressed = true;
            mouse_state.origin = cursor_position;
            mouse_state.delta = Vec2::ZERO;
        }
    }
    if buttons.just_released(MouseButton::Left) {
        let window = windows.get_primary().unwrap();
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
        if let FileDragAndDrop::DroppedFile { id, path_buf } = ev {
            if id.is_primary() {
                // it was dropped over the main window
                let Some(image_absolute) = path_buf.as_path().to_str() else {
                    println!("Can't resolve given path: {:?}", path_buf);
                    continue;
                };
                images_filename.push(String::from(image_absolute));
            }
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

fn get_short_name(path : &String) -> Option<&str> {
    Path::new(path).file_name()?.to_str()
}
