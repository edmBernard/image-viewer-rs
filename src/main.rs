#![allow(unused_variables)]

use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowDescriptor, WindowResized};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    window: WindowDescriptor {
                        title: "I am a window!".to_string(),
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

// fn player_level_up(
//     mut ev_levelup: EventWriter<LevelUpEvent>,
//     query: Query<(Entity, &PlayerXp)>,
// ) {
//     for (entity, xp) in query.iter() {
//         if xp.0 > 1000 {
//             ev_levelup.send(LevelUpEvent(entity));
//         }
//     }
// }

// toggle_resolution(
//     keys: Res<Input<KeyCode>>,
//     mut windows: ResMut<Windows>,
//     resolution: Res<ResolutionSettings>,
// ) {
//     let window = windows.primary_mut();

//     if keys.just_pressed(KeyCode::Key1) {
//         let res = resolution.small;
//         window.set_resolution(res.x, res.y);
//     }
//     if keys.just_pressed(KeyCode::Key2) {
//         let res = resolution.medium;
//         window.set_resolution(res.x, res.y);
//     }
//     if keys.just_pressed(KeyCode::Key3) {
//         let res = resolution.large;
//         window.set_resolution(res.x, res.y);
//     }
// }
fn on_move_image(
    mut ev_move_image: EventReader<MoveImageEvent>,
    windows: Res<Windows>,
    mut sprite_position: Query<(&Id, &mut Transform), With<Handle<Image>>>,
    layout_query: Query<&GridLayout>,
) {
    let layout = layout_query.single();
    let window = windows.primary();

    for ev in ev_move_image.iter() {
        match layout {
            GridLayout::Horizontal => {
                let length = sprite_position.iter().count();
                let step = window.width() / length as f32;
                let offset = -window.width() / 2. + step / 2.;

                for (id, mut transform) in &mut sprite_position {
                    transform.translation.x = id.0 as f32 * step + offset;
                    transform.translation.y = 0.;
                }
            }
            GridLayout::Vertical => {
                let length = sprite_position.iter().count();
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
}

// if let Ok((health, mut transform)) = query.get_mut(entity) {
//     // do something with the components
// } else {
//     // the entity does not have the components from the query
// }
fn on_resize_system(
    resize_reader: EventReader<WindowResized>,
    mut ev_move_image: EventWriter<MoveImageEvent>,
) {
    ev_move_image.send(MoveImageEvent);

    // let layout = layout_query.single();

    // for e in resize_reader.iter() {
    //     match layout {
    //         GridLayout::Horizontal => {
    //             let length = sprite_position.iter().count();
    //             let step = e.width / length as f32;
    //             let offset = -e.width / 2. + step / 2.;

    //             for (id, mut transform) in &mut sprite_position{
    //                 transform.translation.x = id.0 as f32 * step + offset;
    //                 transform.translation.y = 0.;
    //             }
    //         }
    //         GridLayout::Vertical => {
    //             let length = sprite_position.iter().count();
    //             let step = e.height / length as f32;
    //             let offset = -e.height / 2. + step / 2.;

    //             for (id, mut transform) in &mut sprite_position{
    //                 transform.translation.x = 0.;
    //                 transform.translation.y = id.0 as f32 * step + offset;
    //             }
    //         }
    //         GridLayout::Grid => {}
    //     }
    //     // When resolution is being changed
    // }
}

fn change_layout(
    keys: Res<Input<KeyCode>>,
    mut ev_move_image: EventWriter<MoveImageEvent>,
    mut layout_query: Query<&mut GridLayout>,
) {
    if keys.just_pressed(KeyCode::L) {
        println!("Key pressed L");
        for mut layout in &mut layout_query {
            *layout = match *layout {
                GridLayout::Grid => GridLayout::Horizontal,
                GridLayout::Horizontal => GridLayout::Vertical,
                GridLayout::Vertical => GridLayout::Grid,
            };
            ev_move_image.send(MoveImageEvent);
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

        for (index, mut transform) in sprite_position.iter_mut().enumerate() {
            let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
            transform.scale.x *= zoom_factor;
            transform.scale.y *= zoom_factor;
        }
    }
}
