#![allow(unused_variables)]

use std::fs::canonicalize;
use std::path::Path;

use bevy::diagnostic::LogDiagnosticsPlugin;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowDescriptor, WindowResized};
use clap::Parser;

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
        .insert_resource(ImagesFilename(images_filename))
        .add_startup_system(setup)
        .add_event::<MoveImageEvent>()
        .add_system(on_move_image)
        .add_system(on_resize_system)
        .add_system(change_layout)
        .add_system(scroll_events)
        .run();

    Ok(())
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

#[derive(Resource)]
struct ImagesFilename(Vec<String>);

#[derive(Component)]
enum GridLayout {
    Grid,
    Horizontal,
    Vertical,
}

#[derive(Component)]
struct Id(i8);

struct MoveImageEvent;

fn setup(
    mut commands: Commands,
    images_filename: Res<ImagesFilename>,
    asset_server: Res<AssetServer>,
) {
    commands.spawn(Camera2dBundle::default());
    for (index, image) in images_filename.0.iter().enumerate() {
        commands.spawn((
            SpriteBundle {
                texture: asset_server.load(image),
                ..default()
            },
            Id(index as i8),
        ));
    }
    commands.spawn(GridLayout::Horizontal);
}

fn on_move_image(
    _move_image_evr: EventReader<MoveImageEvent>,
    windows: Res<Windows>,
    mut sprite_position: Query<(&Id, &mut Transform), With<Handle<Image>>>,
    layout_query: Query<&GridLayout>,
) {
    let layout = layout_query.single();
    let window = windows.primary();
    let length = sprite_position.iter().count();

    match layout {
        GridLayout::Horizontal => {
            let step = window.width() / length as f32;
            let offset = -window.width() / 2. + step / 2.;

            for (id, mut transform) in &mut sprite_position {
                transform.translation.x = id.0 as f32 * step + offset;
                transform.translation.y = 0.;
            }
        }
        GridLayout::Vertical => {
            let step = window.height() / length as f32;
            let offset = -window.height() / 2. + step / 2.;

            for (id, mut transform) in &mut sprite_position {
                transform.translation.x = 0.;
                transform.translation.y = id.0 as f32 * step + offset;
            }
        }
        GridLayout::Grid => {}
    }
}

fn on_resize_system(
    _resize_evr: EventReader<WindowResized>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
) {
    move_image_evw.send(MoveImageEvent);
}

fn change_layout(
    keys: Res<Input<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut layout_query: Query<&mut GridLayout>,
) {
    if keys.just_pressed(KeyCode::L) {
        for mut layout in &mut layout_query {
            *layout = match *layout {
                GridLayout::Grid => GridLayout::Horizontal,
                GridLayout::Horizontal => GridLayout::Vertical,
                GridLayout::Vertical => GridLayout::Grid,
            };
            move_image_evw.send(MoveImageEvent);
        }
    }
}

fn scroll_events(
    mut scroll_evr: EventReader<MouseWheel>,
    mut sprite_position: Query<&mut Transform, With<Handle<Image>>>,
) {
    use bevy::input::mouse::MouseScrollUnit;
    for ev in scroll_evr.iter() {
        let scroll = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y,
        };

        for mut transform in &mut sprite_position {
            let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
            transform.scale.x *= zoom_factor;
            transform.scale.y *= zoom_factor;
        }
    }
}
