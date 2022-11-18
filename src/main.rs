#![allow(unused_variables)]

use bevy::diagnostic::LogDiagnosticsPlugin;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowDescriptor, WindowResized};

fn main() {
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
        .add_startup_system(setup)
        .add_event::<MoveImageEvent>()
        .add_system(on_move_image)
        .add_system(on_resize_system)
        .add_system(change_layout)
        .add_system(scroll_events)
        .run();
}

#[derive(Component)]
enum GridLayout {
    Grid,
    Horizontal,
    Vertical,
}

#[derive(Component)]
struct Id(i8);

struct MoveImageEvent;

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2dBundle::default());
    let image_list = vec![
        "/Users/ebernard/Documents/tools/image-viewer/image (3).png",
        "/Users/ebernard/Documents/tools/image-viewer/image (3).png",
        "/Users/ebernard/Documents/tools/image-viewer/image (3).png",
    ];
    for (index, image) in image_list.into_iter().enumerate() {
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
