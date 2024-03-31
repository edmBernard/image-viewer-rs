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
use bevy::render::render_asset::RenderAssetUsages;
use bevy::window::{PresentMode, Window, WindowResized};
use bevy_egui::egui::CollapsingHeader;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use clap::Parser;
use home;
use image::{ColorType, DynamicImage, ImageFormat, SubImage};
use serde::Deserialize;
use serde_json;

#[doc(hidden)]
type Result<T> = ::std::result::Result<T, Box<dyn ::std::error::Error>>;

#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    // Images to show
    images: Vec<String>,
}

const HELP_STRING: &'static str = "Keyboard Shortcut:
    L: Change Layout (Grid, Stack, Horizontal, Vertical)
    Double-click: Switch between layout Grid-Stack or Horizontal-Vertical
    R: Rotate images
    Shift + 1, 2, 3, ...: Move Image on top
    Ctrl/Cmd + 1, 2, 3, 4, 5: Zoom by 1, 2, 4, 8, 16
    Ctrl/Cmd + Shift + 1, 2, 3, 4, 5: Zoom by 1/2, 1/4, 1/8, 1/16, 1/32
    Z + Right/Left clic: zoom in/out the hovered image only
    C: Toggle multi cursor
    P: Save image to disk with the displayed crop (prefixed by cr_)
    H: Toggle Interface

    Drag and Drop image from files explorer.
";

// -----------------------------
// Config Struct
#[derive(Deserialize, Debug)]
struct ConfigShortcut {
    save_crop_image: KeyCode,
    local_zoom_modifier: KeyCode,
    switch_cursor: KeyCode,
    switch_layout: KeyCode,
    rotate_images: KeyCode,
}

// Used to store temporary edition during manual edit
#[derive(Default)]
struct ConfigShortcutAsString {
    save_crop_image: Option<String>,
    local_zoom_modifier: Option<String>,
    switch_cursor: Option<String>,
    switch_layout: Option<String>,
    rotate_images: Option<String>,
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
    zoom_on_scroll_enabled: bool,
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

        let Some(config_str) = std::fs::read_to_string(home_directory.join(config_filename)).ok() else {
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
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Image Viewer 3000".to_string(),
                        resolution: [700., 300.].into(),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
            EguiPlugin,
        ))
        .insert_resource(InitialImagesFilename(images_filename))
        .insert_resource(UiState { visible: true })
        .insert_resource(config_data)
        .insert_resource(GlobalScale(1. / 8.))
        .insert_resource(MultiCursorEnabled(false))
        .insert_resource(GridLayoutState {
            layout: GridLayout::Grid,
            index: 0,
        })
        .insert_resource(MouseState {
            origin: Vec2::ZERO,
            delta: Vec2::ZERO,
            pressed: false,
        })
        .add_systems(Startup, setup)
        .add_systems(Startup, configure_visuals)
        .add_event::<LoadNewImageEvent>()
        .add_event::<NewImageLoadedEvent>()
        .add_event::<MoveImageEvent>()
        .add_event::<ToggleCursor>()
        .add_event::<ResetVisibilityEvent>()
        .add_systems(Update, ui_example)
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
        .add_systems(Update, key_toggle_cursor)
        .add_systems(Update, toggle_cursor)
        .add_systems(Update, save_cropped)
        .run();

    Ok(())
}

// -----------------------------
// State Struct
#[derive(Debug, Resource)]
struct MultiCursorEnabled(bool);

#[derive(Resource)]
struct UiState {
    visible: bool,
}

#[derive(PartialEq, Debug)]
enum GridLayout {
    Stack,
    Horizontal,
    Vertical,
    Grid,
}

#[derive(Resource)]
struct GridLayoutState {
    layout: GridLayout,
    index: usize,
}

#[derive(Resource)]
struct GlobalScale(f32);

#[derive(Resource)]
struct MouseState {
    origin: Vec2,
    delta: Vec2,
    pressed: bool,
}

#[derive(Component)]
struct FontHandle(Handle<Font>);

#[derive(Resource)]
struct InitialImagesFilename(Vec<String>);

// -----------------------------
// Components
#[derive(Component)]
struct Id(usize);

#[derive(Component)]
struct Scale(f32);

#[derive(Component)]
struct Position(Vec2);

// Rotation in quarter turn (1 is 1 turn)
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

// -----------------------------
// Events
#[derive(Event)]
struct MoveImageEvent;

#[derive(Event)]
struct ToggleCursor;

#[derive(Event)]
struct ResetVisibilityEvent;

#[derive(Event)]
struct NewImageLoadedEvent {
    handle: Handle<Image>,
    path: String,
    index: usize,
    count: usize,
}

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
        .with_text_justify(JustifyText::Left)
        .with_style(Style {
            position_type: PositionType::Absolute,
            top: Val::Px(5.),
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

fn configure_visuals(mut egui_ctx: EguiContexts) {
    egui_ctx.ctx_mut().set_visuals(egui::Visuals { ..Default::default() });
}

fn keycode_dropdown(ui: &mut egui::Ui, label: &str, current_key: &mut KeyCode, ongoing: &mut Option<String>) {
    ui.horizontal(|ui| {
        ui.label(label);

        let Ok(key_previous) = serde_json::to_string(&current_key) else {
            panic!("Should not happend: Can't convert KeyCode to string");
        };
        let mut key_string = match ongoing {
            Some(value) => value.clone(),
            None => {
                let string_len = key_previous.len();
                // strip quote
                String::from(&key_previous[1..string_len - 1])
            }
        };
        ui.text_edit_singleline(&mut key_string);

        let key_json = format!("\"{}\"", key_string);
        if let Ok(key) = serde_json::from_str::<KeyCode>(&key_json) {
            *current_key = key;
        }
        *ongoing = Some(key_string);
    });
}

fn ui_example(
    mut contexts: EguiContexts,
    mut config: ResMut<Config>,
    mut layout_state: ResMut<GridLayoutState>,
    mut ongoing_edit: Local<ConfigShortcutAsString>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
    mut ui_state: ResMut<UiState>,
    mut ui_panel_visible: Local<bool>,
    mut global_scale: ResMut<GlobalScale>,
    mut cursor_state: ResMut<MultiCursorEnabled>,
    mut cursor_evw: EventWriter<ToggleCursor>,
) {
    if ui_state.visible {
        egui::TopBottomPanel::bottom("wrap_app_top_bar").show(contexts.ctx_mut(), |ui| {
            // equivalent to horizontal_wrapped but with a small factor on y to avoid the clip of button
            let initial_size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y * 1.2);
            ui.allocate_ui_with_layout(
                initial_size,
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    egui::widgets::global_dark_light_mode_switch(ui);
                    ui.toggle_value(&mut *ui_panel_visible, "Settings");
                    ui.separator();
                    let mut scale = global_scale.0.log2();
                    if ui
                        .add(egui::DragValue::new(&mut scale).speed(0.1).clamp_range(-10.0..=10.))
                        .on_hover_text("Zoom")
                        .changed()
                    {
                        global_scale.0 = 2f32.powf(scale);
                    }
                    ui.separator();
                    for i in 0..10 {
                        let mut state = i == layout_state.index;
                        if ui.toggle_value(&mut state, format!("{}", i + 1)).changed() {
                            layout_state.index = i;
                            reset_vix_evw.send(ResetVisibilityEvent);
                        }
                    }

                    ui.separator();
                    let elem1 = ui
                        .selectable_value(&mut layout_state.layout, GridLayout::Grid, "Grid")
                        .changed();
                    let elem2 = ui
                        .selectable_value(&mut layout_state.layout, GridLayout::Stack, "Stack")
                        .changed();
                    let elem3 = ui
                        .selectable_value(&mut layout_state.layout, GridLayout::Horizontal, "Horizontal")
                        .changed();
                    let elem4 = ui
                        .selectable_value(&mut layout_state.layout, GridLayout::Vertical, "Vertical")
                        .changed();
                    if elem1 || elem2 || elem3 || elem4 {
                        reset_vix_evw.send(ResetVisibilityEvent);
                    }
                },
            );
        });

        if *ui_panel_visible {
            egui::SidePanel::right("Settings")
                .resizable(false)
                .show(contexts.ctx_mut(), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("Settings");
                        ui.hyperlink_to("Source Code", "https://github.com/edmBernard/image-viewer-rs");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                        ui.checkbox(&mut config.misc.zoom_on_scroll_enabled, "Enable Zoom on Scroll");

                        if ui.checkbox(&mut cursor_state.0, "Enable Multi Cursor").changed() {
                            cursor_evw.send(ToggleCursor);
                        };

                        CollapsingHeader::new("Style").default_open(true).show(ui, |ui| {
                            ui.label("New style is applied when new images are loaded");
                            ui.horizontal(|ui| {
                                ui.label("Font Size:");
                                ui.add(egui::Slider::new(&mut config.text.font_size, 8.0..=70.0));
                            });
                            let mut color_vec = config.text.font_color.rgba_linear_to_vec4().to_array();
                            ui.horizontal(|ui| {
                                ui.label("Font Color:");
                                ui.color_edit_button_rgba_unmultiplied(&mut color_vec);
                                ui.label(format!(
                                    "rgba: ({:.2}, {:.2}, {:.2}, {:.2})",
                                    color_vec[0], color_vec[1], color_vec[2], color_vec[3],
                                ));
                            });
                            config.text.font_color = Color::rgba_linear_from_array(color_vec);
                        });

                        CollapsingHeader::new("Short Cut").default_open(true).show(ui, |ui| {
                            keycode_dropdown(
                                ui,
                                "Save Crop:",
                                &mut config.shortcut.save_crop_image,
                                &mut ongoing_edit.save_crop_image,
                            );
                            keycode_dropdown(
                                ui,
                                "Local Zoom Modifier:",
                                &mut config.shortcut.local_zoom_modifier,
                                &mut ongoing_edit.local_zoom_modifier,
                            );
                            keycode_dropdown(
                                ui,
                                "Change Cursor:",
                                &mut config.shortcut.switch_cursor,
                                &mut ongoing_edit.switch_cursor,
                            );
                            keycode_dropdown(
                                ui,
                                "Rotate:",
                                &mut config.shortcut.rotate_images,
                                &mut ongoing_edit.rotate_images,
                            );
                            keycode_dropdown(
                                ui,
                                "Change Layout:",
                                &mut config.shortcut.switch_layout,
                                &mut ongoing_edit.switch_layout,
                            );
                        });
                    });
                });
        }
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
                let new_image = Image::from_dynamic(
                    image,
                    true,
                    RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                );
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            ColorType::Rgb16 | ColorType::Rgba16 => {
                let new_image = Image::from_dynamic(
                    image,
                    true,
                    RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                );
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
                let new_image = Image::from_dynamic(
                    image_rgb16,
                    true,
                    RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                );
                let handle = images.add(new_image);
                loaded_evw.send(NewImageLoadedEvent {
                    handle: handle,
                    path: ev.path.clone(),
                    index: ev.index,
                    count: ev.count,
                });
            }
            _ => {
                println!("Unsupported image type : image.color(): {:?}", image.color())
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
    mut image_loaded: Local<usize>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
    layout_state: Res<GridLayoutState>,
) {
    for ev in load_image_evr.read() {
        let font = font_query.single();

        if *image_loaded == 0 {
            for entity in &images {
                commands.entity(entity).despawn();
            }
        }
        println!("image loaded count : {}", *image_loaded);
        let visibility = match layout_state.layout {
            GridLayout::Stack => {
                if *image_loaded != layout_state.index {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                }
            }
            _ => Visibility::Visible,
        };

        *image_loaded += 1;
        if *image_loaded >= ev.count {
            *image_loaded = 0;
        }

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
            .with_text_justify(JustifyText::Left),
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
    global_scale: Res<GlobalScale>,
    layout_state: Res<GridLayoutState>,
    mouse_state: Res<MouseState>,
    mut title_query: Query<&mut Style, With<MyText>>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let window = windows.single();
    let num_images = sprite_position.iter().count();
    for (id, image_handle, position, scale, rotation, mut transform, mut sprite) in &mut sprite_position {
        let Some(image) = assets.get(image_handle) else {
            continue;
        };
        let image_size = image.size().as_vec2();

        let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, &layout_state.layout, window);
        transform.translation = (Vec2::new(-window.width() / 2., -window.height() / 2.) + cell_offset + cell_size / 2.)
            .extend(transform.translation.z)
            * Vec3::new(1., -1., 1.);
        transform.scale = Vec2::splat(scale.0 * global_scale.0).extend(1.);
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation.0);

        let delta =
            Vec2::from_angle(PI / 2. * rotation.0).rotate(position.0 + mouse_state.delta / (scale.0 * global_scale.0));
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

    let (_, cell_size) = get_cell_rect(0, num_images, &layout_state.layout, window);
    for mut style in &mut title_query {
        style.width = Val::Px(cell_size.x);
    }
}

fn on_move_image_title(
    mut move_image_evr: EventReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&Id, &mut Style), With<MyText>>,
    layout_state: Res<GridLayoutState>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let num_images = text_query.iter().count();
    let window = windows.single();

    for (id, mut style) in &mut text_query {
        let (cell_offset, _) = get_cell_rect(id.0, num_images, &layout_state.layout, window);
        style.top = Val::Px(cell_offset.y + 2.);
        style.left = Val::Px(cell_offset.x + 5.);
    }
}

fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&Id, &mut Transform), With<MyCursor>>,
    layout_state: Res<GridLayoutState>,
) {
    let num_images = cursor_query.iter().count();
    let window = windows.single();

    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    for (id, mut transform) in &mut cursor_query {
        let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, &layout_state.layout, window);
        let new_y = cell_offset.y + f32::rem_euclid(cursor_position.y, cell_size.y);
        let new_x = cell_offset.x + f32::rem_euclid(cursor_position.x, cell_size.x);
        transform.translation = Vec3::new(
            -window.width() / 2. + new_x,
            window.height() / 2. - new_y,
            transform.translation.z,
        );
    }
}

fn on_resize_system(mut resize_evr: EventReader<WindowResized>, mut move_image_evw: EventWriter<MoveImageEvent>) {
    for _ in resize_evr.read() {
        move_image_evw.send(MoveImageEvent);
    }
}

fn on_reset_visibility(
    mut reset_evr: EventReader<ResetVisibilityEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    layout_state: Res<GridLayoutState>,
) {
    for _ in reset_evr.read() {
        for (i, mut visibility) in &mut visibility_query {
            *visibility = match layout_state.layout {
                GridLayout::Stack => {
                    if i.0 == layout_state.index {
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
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    if keys.just_pressed(config.shortcut.switch_layout) {
        layout_state.layout = match layout_state.layout {
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
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut reset_vix_evw: EventWriter<ResetVisibilityEvent>,
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

        layout_state.layout = match layout_state.layout {
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
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut visibility_query: Query<(&Id, &mut Visibility)>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl_pressed {
        return;
    }
    let shift_pressed = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    layout_state.index = if shift_pressed && keys.just_pressed(KeyCode::Digit1) {
        1
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit2) {
        2
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit3) {
        3
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit4) {
        4
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit5) {
        5
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit6) {
        6
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit7) {
        7
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit8) {
        8
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit9) {
        9
    } else if shift_pressed && keys.just_pressed(KeyCode::Digit0) {
        10
    } else {
        return;
    };
    layout_state.index -= 1;

    for (i, mut visibility) in &mut visibility_query {
        *visibility = match layout_state.layout {
            GridLayout::Stack => {
                if i.0 == layout_state.index {
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
    keys: Res<ButtonInput<KeyCode>>,
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
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut global_scale: ResMut<GlobalScale>,
) {
    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift_pressed = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let scale_factor = if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Digit1) {
        -1.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Digit2) {
        -2.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Digit3) {
        -3.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Digit4) {
        -4.
    } else if ctrl_pressed && shift_pressed && keys.just_pressed(KeyCode::Digit5) {
        -5.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit1) {
        0.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit2) {
        1.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit3) {
        3.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit4) {
        4.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit5) {
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
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    mut sprite_query: Query<(&Id, &mut Scale, &mut Position), With<MyImage>>,
) {
    if keys.pressed(config.shortcut.local_zoom_modifier) {
        if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
            return;
        }

        let num_images = sprite_query.iter().count();
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
                let (cell_offset, cell_size) = get_cell_rect(id.0, num_images, &layout_state.layout, window);

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

fn get_cell_rect(index: usize, num_images: usize, layout: &GridLayout, window: &Window) -> (Vec2, Vec2) {
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

fn toggle_help(keys: Res<ButtonInput<KeyCode>>, mut ui_state: ResMut<UiState>) {
    if keys.just_pressed(KeyCode::KeyH) {
        ui_state.visible = !ui_state.visible;
    }
}

fn key_toggle_cursor(keys: Res<ButtonInput<KeyCode>>, config: Res<Config>, mut toggle_evw: EventWriter<ToggleCursor>) {
    if keys.just_pressed(config.shortcut.switch_cursor) {
        toggle_evw.send(ToggleCursor);
    }
}

fn toggle_cursor(
    mut toggle_evr: EventReader<ToggleCursor>,
    mut commands: Commands,
    cursor_query: Query<Entity, With<MyCursor>>,
    mut cursor_state: ResMut<MultiCursorEnabled>,
    image_query: Query<&Id, With<MyImage>>,
) {
    for _ev in toggle_evr.read() {
        if cursor_query.iter().count() == 0 {
            *cursor_state = MultiCursorEnabled(true);
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
            *cursor_state = MultiCursorEnabled(false);
            for entity in &cursor_query {
                commands.entity(entity).despawn_recursive();
            }
        }
    }
}

fn save_cropped(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
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
    mut global_scale: ResMut<GlobalScale>,
) {
    if config.misc.zoom_on_scroll_enabled {
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
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut mouse_state: ResMut<MouseState>,
    global_scale: Res<GlobalScale>,
    mut image_query: Query<(&mut Position, &Scale), With<MyImage>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let window = windows.single();
        let Some(cursor_position) = window.cursor_position() else {
            return;
        };
        mouse_state.pressed = true;
        mouse_state.origin = cursor_position;
        mouse_state.delta = Vec2::ZERO;
    }
    if buttons.just_released(MouseButton::Left) {
        mouse_state.pressed = false;

        let window = windows.single();

        let Some(cursor_position) = window.cursor_position() else {
            // Cursor is outside of the windows
            for (mut position, scale) in &mut image_query {
                position.0 += mouse_state.delta / (scale.0 * global_scale.0);
                let delta = mouse_state.delta;
                mouse_state.origin += delta / (scale.0 * global_scale.0);
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

fn cursor_move(
    mut cursor_evr: EventReader<CursorMoved>,
    mut move_image_evw: EventWriter<MoveImageEvent>,
    mut mouse_state: ResMut<MouseState>,
) {
    for ev in cursor_evr.read() {
        if mouse_state.pressed {
            mouse_state.delta = ev.position - mouse_state.origin;
            move_image_evw.send(MoveImageEvent);
        }
    }
}

fn file_drop(mut dnd_evr: EventReader<FileDragAndDrop>, mut load_image_evw: EventWriter<LoadNewImageEvent>) {
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
    Vec2::new(vec.x.clamp(rect.min.x, rect.max.x), vec.y.clamp(rect.min.y, rect.max.y))
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
