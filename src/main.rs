// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

use std::f32::consts::{PI, TAU};
use std::fs::canonicalize;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io::BufWriter;
use std::path::Path;
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy::window::{PresentMode, WindowResized, WindowResolution};
use bevy_egui::egui::CollapsingHeader;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use clap::Parser;
use image::{ColorType, DynamicImage, ImageFormat, SubImage};
use serde::{Deserialize, Serialize};

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
    Shift + 1, 2, 3, ...: Move Image on top
    Ctrl/Cmd + 1, 2, 3, 4, 5: Zoom by 1, 2, 4, 8, 16
    Ctrl/Cmd + Shift + 1, 2, 3, 4, 5: Zoom by 1/2, 1/4, 1/8, 1/16, 1/32
    Z + Right/Left clic: zoom in/out the hovered image only
    R: Rotate all images CW
    E + Right/Left clic: rotate CW/CCW the hovered image only
    A + drop file : Add new images to the comparison
    C: Toggle multi cursor
    P: Save image to disk with the displayed crop (suffixed by _crop)
    H: Toggle Interface

    Drag and Drop image from files explorer.
";

#[derive(States, Debug, Clone, PartialEq, Eq, Hash)]
enum MyAppState {
    Working,
    EditShortCut,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum ScrollBehavior {
    Zoom,
    Move,
    None,
}

// -----------------------------
// Config Struct
#[derive(Serialize, Deserialize, Debug)]
struct ConfigShortcut {
    save_crop_image: KeyCode,
    local_zoom_modifier: KeyCode,
    local_rotate_modifier: KeyCode,
    switch_cursor: KeyCode,
    switch_layout: KeyCode,
    rotate_images: KeyCode,
    add_images: KeyCode,
}

// Used to store temporary edition during manual edit
#[derive(Default)]
struct ConfigShortcutAsBool {
    save_crop_image: bool,
    local_zoom_modifier: bool,
    local_rotate_modifier: bool,
    switch_cursor: bool,
    switch_layout: bool,
    rotate_images: bool,
    add_images: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct ConfigText {
    font_size: f32,
    font_color: Color,
}

#[derive(Serialize, Deserialize, Debug)]
struct ConfigHDR {
    enabled: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct ConfigMisc {
    scroll_behavior: ScrollBehavior,
    grid_width: i32,
}

#[derive(Serialize, Deserialize, Debug, Resource)]
struct Config {
    text: ConfigText,
    shortcut: ConfigShortcut,
    hdr: ConfigHDR,
    misc: ConfigMisc,
}

// MARK: Main
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

        let Ok(config) = toml::from_str(&config_str) else {
            break 'block None;
        };

        config
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
                        resolution: WindowResolution::new(800, 350),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
            EguiPlugin::default(),
        ))
        .insert_state(MyAppState::Working)
        .insert_resource(InitialImagesFilename(images_filename))
        .insert_resource(UiState {
            visible: true,
            settings_panel_visible: false,
        })
        .insert_resource(config_data)
        .insert_resource(GlobalScale(1. / 8.))
        .insert_resource(GlobalRotation(0.))
        .insert_resource(NewImageBatch(true))
        .insert_resource(MultiCursorEnabled(false))
        .insert_resource(RecordedPressedKey(None))
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
        .add_message::<LoadNewImageEvent>()
        .add_message::<NewImageLoadedEvent>()
        .add_message::<MoveImageEvent>()
        .add_message::<ToggleCursor>()
        .add_message::<SaveCropped>()
        .add_message::<ResetVisibilityEvent>()
        .add_message::<ResetScales>()
        .add_message::<FitToScreen>()
        .add_message::<ChangeTitleStyleEvent>()
        .add_message::<SaveSettingsEvent>()
        // Egui systems must run in EguiPrimaryContextPass (not Update)
        .add_systems(
            EguiPrimaryContextPass,
            configure_visuals.run_if(run_once),
        )
        .add_systems(
            EguiPrimaryContextPass,
            (
                ui_bottom_menu,
                ui_settings_menu.after(ui_bottom_menu),
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(
            EguiPrimaryContextPass,
            ui_edit_short_cut.run_if(in_state(MyAppState::EditShortCut)),
        )
        // Non-egui systems in Update
        .add_systems(
            Update,
            (
                key_change_layout,
                change_layout_on_click,
                change_global_zoom,
                change_zoom_individually,
                change_rotation_individually,
                scroll_events,
                mouse_button_input,
                cursor_move,
                file_drop,
                on_reset_visibility,
                on_resize_system,
                on_image_loaded,
                on_move_cursor,
                on_move_image,
                on_move_image_title,
                on_load_image,
                on_image_spawned,
                toggle_help,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        // Bevy doesn't allow more than 20 systems in the declaration of anonymous system set
        // https://docs.rs/bevy/latest/bevy/prelude/trait.IntoScheduleConfigs.html#foreign-impls
        // That should really be in the documentation of `add_systems` method
        // Seriously :face_palm: Is it Bevy or Rust fault ?
        // Edit: nice it's now in the documentation
        .add_systems(
            Update,
            (
                change_top_image,
                change_global_rotation,
                key_toggle_cursor,
                toggle_cursor,
                reset_scales,
                fit_to_screen,
                key_save_cropped,
                save_cropped,
                save_settings,
                change_image_title_style,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(
            Update,
            record_pressed_key.run_if(in_state(MyAppState::EditShortCut)),
        )
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
    settings_panel_visible: bool,
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
struct GlobalRotation(f32);

#[derive(Resource)]
struct NewImageBatch(bool);

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

#[derive(Resource, Debug)]
struct RecordedPressedKey(Option<KeyCode>);

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
// Messages (buffered events)
#[derive(Message)]
struct MoveImageEvent;

#[derive(Message)]
struct ChangeTitleStyleEvent;

#[derive(Message)]
struct SaveSettingsEvent;

#[derive(Message)]
struct ToggleCursor;

#[derive(Message)]
struct SaveCropped;

#[derive(Message)]
struct ResetScales;

#[derive(Message)]
struct FitToScreen;

#[derive(Message)]
struct ResetVisibilityEvent;

#[derive(Message)]
struct NewImageLoadedEvent {
    handle: Handle<Image>,
    path: String,
    index: usize,
}

#[derive(Message)]
struct LoadNewImageEvent {
    path: String,
    index: usize,
}

// MARK: Setup
fn setup(
    mut commands: Commands,
    images_filename: ResMut<InitialImagesFilename>,
    config: Res<Config>,
    mut load_image_evw: MessageWriter<LoadNewImageEvent>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let mut camera = commands.spawn(Camera2d);
    if config.hdr.enabled {
        camera.insert(Hdr);
    }

    let bytes = include_bytes!("../assets/fonts/IBMPlexMono-Regular.otf");
    let font = Font::try_from_bytes(bytes.to_vec()).unwrap();
    let font_handle = fonts.add(font);
    commands.spawn(FontHandle(font_handle.clone()));

    commands.spawn((
        Text::new(HELP_STRING),
        TextFont {
            font: font_handle,
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::Srgba(bevy::color::palettes::css::ANTIQUE_WHITE)),
        TextLayout {
            justify: Justify::Left,
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(5.),
            left: Val::Px(5.),
            ..default()
        },
        MyHelp,
    ));

    for (index, image) in images_filename.0.iter().enumerate() {
        load_image_evw.write(LoadNewImageEvent {
            path: image.clone(),
            index: index,
        });
    }
}

fn configure_visuals(mut egui_ctx: EguiContexts) {
    let Ok(ctx) = egui_ctx.ctx_mut() else { return };
    ctx.set_visuals(egui::Visuals { ..Default::default() });
}

fn keycode_dropdown(
    ui: &mut egui::Ui,
    next_state: &mut ResMut<NextState<MyAppState>>,
    label: &str,
    current_key: &mut KeyCode,
    ongoing: &mut bool,
    recorded_key: &mut ResMut<RecordedPressedKey>,
) {
    ui.horizontal(|ui| {
        ui.label(label);

        let key_previous = format!("{current_key:?}");
        let response = ui.toggle_value(ongoing, &key_previous);
        if response.changed() {
            if *ongoing {
                next_state.set(MyAppState::EditShortCut);
            }
        }
        if *ongoing && recorded_key.0.is_some() {
            *ongoing = false;
            *current_key = recorded_key.0.unwrap();
            recorded_key.0 = None;
        }
    });
}

fn ui_bottom_menu(
    mut contexts: EguiContexts,
    mut layout_state: ResMut<GridLayoutState>,
    mut reset_vix_evw: MessageWriter<ResetVisibilityEvent>,
    mut ui_state: ResMut<UiState>,
    mut global_scale: ResMut<GlobalScale>,
    mut save_cropped_evw: MessageWriter<SaveCropped>,
    mut reset_scales_evw: MessageWriter<ResetScales>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
) {
    if ui_state.visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        egui::TopBottomPanel::bottom("wrap_app_top_bar").show(ctx, |ui| {
            // equivalent to horizontal_wrapped but with a small factor on y to avoid the clip of button
            let initial_size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y * 1.2);
            ui.allocate_ui_with_layout(
                initial_size,
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    egui::widgets::global_theme_preference_switch(ui);
                    ui.toggle_value(&mut ui_state.settings_panel_visible, "Settings");
                    ui.separator();
                    let mut scale = global_scale.0.log2();

                    if ui
                        .add(
                            egui::DragValue::new(&mut scale)
                                .prefix("\u{1F50E} ")
                                .speed(0.1)
                                .range(-10.0..=10.),
                        )
                        .on_hover_text("Zoom")
                        .changed()
                    {
                        global_scale.0 = 2f32.powf(scale);
                    }

                    if ui.button("1:1").on_hover_text("Reset All Zoom").clicked() {
                        reset_scales_evw.write(ResetScales);
                    }

                    if ui.button("Fit").on_hover_text("Fit to Screen").clicked() {
                        fit_to_screen_evw.write(FitToScreen);
                    }

                    for i in 0..10 {
                        let mut state = i == layout_state.index;
                        if ui.toggle_value(&mut state, format!("{}", i + 1)).changed() {
                            layout_state.index = i;
                            reset_vix_evw.write(ResetVisibilityEvent);
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
                        .selectable_value(&mut layout_state.layout, GridLayout::Vertical, "Vertical")
                        .changed();
                    let elem4 = ui
                        .selectable_value(&mut layout_state.layout, GridLayout::Horizontal, "Horizontal")
                        .changed();
                    if elem1 || elem2 || elem3 || elem4 {
                        reset_vix_evw.write(ResetVisibilityEvent);
                    }

                    ui.separator();
                    if ui
                        .button("\u{26F6}")
                        .on_hover_text("Save crops next to original images suffixed with _crop")
                        .clicked()
                    {
                        save_cropped_evw.write(SaveCropped);
                    }
                },
            );
        });
    }
}

fn ui_settings_menu(
    mut contexts: EguiContexts,
    mut config: ResMut<Config>,
    mut ongoing_edit: Local<ConfigShortcutAsBool>,
    mut recorded_key: ResMut<RecordedPressedKey>,
    ui_state: Res<UiState>,
    mut cursor_state: ResMut<MultiCursorEnabled>,
    mut cursor_evw: MessageWriter<ToggleCursor>,
    mut change_title_style_evw: MessageWriter<ChangeTitleStyleEvent>,
    mut save_settings_evw: MessageWriter<SaveSettingsEvent>,
    mut next_state: ResMut<NextState<MyAppState>>,
) {
    if ui_state.settings_panel_visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        egui::SidePanel::right("Settings").resizable(false).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Settings");
                ui.hyperlink_to(
                    format!("{} Source Code", egui::special_emojis::GITHUB),
                    "https://github.com/edmBernard/image-viewer-rs",
                );
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Behavior on Scroll:");
                    egui::ComboBox::from_label("")
                        .selected_text(format!("{:?}", config.misc.scroll_behavior))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::None, "Disabled");
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::Move, "Move");
                            ui.selectable_value(&mut config.misc.scroll_behavior, ScrollBehavior::Zoom, "Zoom");
                        });
                });

                if ui.checkbox(&mut cursor_state.0, "Enable Multi Cursor").changed() {
                    cursor_evw.write(ToggleCursor);
                };

                ui.horizontal(|ui| {
                    ui.label("Grid Width:");
                    ui.add(egui::DragValue::new(&mut config.misc.grid_width));
                });

                CollapsingHeader::new("Style").default_open(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Font Size:");
                        if ui
                            .add(egui::Slider::new(&mut config.text.font_size, 8.0..=70.0))
                            .changed()
                        {
                            change_title_style_evw.write(ChangeTitleStyleEvent);
                        }
                    });
                    let mut color_vec = config.text.font_color.to_linear().to_f32_array();
                    ui.horizontal(|ui| {
                        ui.label("Font Color:");
                        if ui.color_edit_button_rgba_unmultiplied(&mut color_vec).changed() {
                            change_title_style_evw.write(ChangeTitleStyleEvent);
                        }
                        ui.label(format!(
                            "rgba: ({:.2}, {:.2}, {:.2}, {:.2})",
                            color_vec[0], color_vec[1], color_vec[2], color_vec[3],
                        ));
                    });
                    config.text.font_color = Color::LinearRgba(LinearRgba::from_f32_array(color_vec));
                });

                CollapsingHeader::new("Short Cut").default_open(true).show(ui, |ui| {
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Save Crop:",
                        &mut config.shortcut.save_crop_image,
                        &mut ongoing_edit.save_crop_image,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Local Zoom Modifier:",
                        &mut config.shortcut.local_zoom_modifier,
                        &mut ongoing_edit.local_zoom_modifier,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Local Rotate Modifier:",
                        &mut config.shortcut.local_rotate_modifier,
                        &mut ongoing_edit.local_rotate_modifier,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Change Cursor:",
                        &mut config.shortcut.switch_cursor,
                        &mut ongoing_edit.switch_cursor,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Rotate:",
                        &mut config.shortcut.rotate_images,
                        &mut ongoing_edit.rotate_images,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Add Images:",
                        &mut config.shortcut.add_images,
                        &mut ongoing_edit.add_images,
                        &mut recorded_key,
                    );
                    keycode_dropdown(
                        ui,
                        &mut next_state,
                        "Change Layout:",
                        &mut config.shortcut.switch_layout,
                        &mut ongoing_edit.switch_layout,
                        &mut recorded_key,
                    );
                });

                if ui.button("Save Settings").clicked() {
                    save_settings_evw.write(SaveSettingsEvent);
                };
            });
        });
    }
}

fn ui_edit_short_cut(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
            ui.heading("Press short cut key");
        })
    });
}

fn on_load_image(
    mut load_evr: MessageReader<LoadNewImageEvent>,
    mut loaded_evw: MessageWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
) {
    for ev in load_evr.read() {
        let Some(f) = File::open(&ev.path).ok() else {
            println!("Failed to open file: {}", ev.path);
            continue;
        };
        let Some(format) = ImageFormat::from_path(&ev.path).ok() else {
            println!("Failed to deduce image format from path: {}", ev.path);
            continue;
        };

        let buf = BufReader::new(f);
        let mut reader = image::ImageReader::with_format(buf, format);

        // Remove the memory limit on image size we can read
        reader.no_limits();

        let Some(image) = reader.decode().ok() else {
            println!("Failed to decode image: {}", ev.path);
            continue;
        };

        let loaded_image = match image.color() {
            ColorType::Rgb8 | ColorType::Rgba8 | ColorType::L8 | ColorType::La8 => Image::from_dynamic(
                image,
                true,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            ),
            ColorType::Rgb16 | ColorType::Rgba16 => Image::from_dynamic(
                image,
                true,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            ),
            ColorType::L16 => {
                let image_rgb16 = DynamicImage::ImageRgb16(image.into_rgb16());
                Image::from_dynamic(
                    image_rgb16,
                    true,
                    RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                )
            }
            _ => {
                println!("Unsupported image type : image.color(): {:?}", image.color());
                continue;
            }
        };
        let handle = images.add(loaded_image);
        loaded_evw.write(NewImageLoadedEvent {
            handle: handle,
            path: ev.path.clone(),
            index: ev.index,
        });
    }
}

fn on_image_loaded(
    config: Res<Config>,
    mut load_image_evr: MessageReader<NewImageLoadedEvent>,
    mut commands: Commands,
    images: Query<Entity, With<Id>>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
    layout_state: Res<GridLayoutState>,
    mut is_new_batch: ResMut<NewImageBatch>,
) {
    for ev in load_image_evr.read() {
        let font = font_query.single().unwrap();

        if is_new_batch.0 {
            for entity in &images {
                commands.entity(entity).despawn();
            }
            is_new_batch.0 = false;
        }

        let visibility = match layout_state.layout {
            GridLayout::Stack => {
                if ev.index != layout_state.index {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                }
            }
            _ => Visibility::Visible,
        };

        commands.spawn((
            Sprite {
                image: ev.handle.clone(),
                ..default()
            },
            visibility,
            Id(ev.index),
            Scale(1.),
            Position(Vec2::ZERO),
            Rotation(0.),
            ImagePath(ev.path.clone()),
            MyImage,
        ));

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            Text::new(short_path),
            TextFont {
                font: font.0.clone(),
                font_size: config.text.font_size,
                ..default()
            },
            TextColor(config.text.font_color),
            TextLayout {
                justify: Justify::Left,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            visibility,
            Id(ev.index),
            MyText,
        ));

        let mut visibility = help_query.single_mut().unwrap();
        *visibility = Visibility::Hidden;
    }
}

fn on_move_image(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    assets: Res<Assets<Image>>,
    mut sprite_position: Query<(&Id, &Position, &Scale, &Rotation, &mut Transform, &mut Sprite), With<MyImage>>,
    global_scale: Res<GlobalScale>,
    global_rotation: Res<GlobalRotation>,
    layout_state: Res<GridLayoutState>,
    mouse_state: Res<MouseState>,
    mut title_query: Query<&mut Node, With<MyText>>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let window = windows.single().unwrap();
    let num_images = sprite_position.iter().count();
    for (id, position, scale, rotation, mut transform, mut sprite) in &mut sprite_position {
        let image_handle = sprite.image.clone();
        let Some(image) = assets.get(&image_handle) else {
            continue;
        };
        let image_size = image.size().as_vec2();

        let (cell_offset, cell_size) =
            get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);
        transform.translation = (Vec2::new(-window.width() / 2., -window.height() / 2.) + cell_offset + cell_size / 2.)
            .extend(transform.translation.z)
            * Vec3::new(1., -1., 1.);
        transform.scale = Vec2::splat(scale.0 * global_scale.0).extend(1.);
        let rotation_total = global_rotation.0 + rotation.0;
        transform.rotation = Quat::from_rotation_z(-TAU / 4. * rotation_total);

        let delta = Vec2::from_angle(-PI / 2. * rotation_total)
            .rotate(position.0 + mouse_state.delta / (scale.0 * global_scale.0));
        let image_crop = Rect::from_center_size(image_size / 2., image_size);
        let rotated_cell_size = if rotation_total % 2. == 0. {
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

    let (_, cell_size) = get_cell_rect(0, num_images, &layout_state.layout, window, config.misc.grid_width);
    for mut node in &mut title_query {
        node.width = Val::Px(cell_size.x);
    }
}

fn on_move_image_title(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&Id, &mut Node), With<MyText>>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let num_images = text_query.iter().count();
    let window = windows.single().unwrap();

    for (id, mut node) in &mut text_query {
        let (cell_offset, _) = get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);
        node.top = Val::Px(cell_offset.y + 2.);
        node.left = Val::Px(cell_offset.x + 5.);
    }
}

fn change_image_title_style(
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
        text_font.font_size = config.text.font_size;
    }
}

fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&Id, &mut Transform), With<MyCursor>>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
) {
    let num_images = cursor_query.iter().count();
    let window = windows.single().unwrap();

    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    for (id, mut transform) in &mut cursor_query {
        let (cell_offset, cell_size) =
            get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);
        let new_y = cell_offset.y + f32::rem_euclid(cursor_position.y, cell_size.y);
        let new_x = cell_offset.x + f32::rem_euclid(cursor_position.x, cell_size.x);
        transform.translation = Vec3::new(
            -window.width() / 2. + new_x,
            window.height() / 2. - new_y,
            transform.translation.z,
        );
    }
}

fn on_resize_system(mut resize_evr: MessageReader<WindowResized>, mut move_image_evw: MessageWriter<MoveImageEvent>) {
    for _ in resize_evr.read() {
        move_image_evw.write(MoveImageEvent);
    }
}

fn on_reset_visibility(
    mut reset_evr: MessageReader<ResetVisibilityEvent>,
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

fn key_change_layout(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut reset_vix_evw: MessageWriter<ResetVisibilityEvent>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    if keys.just_pressed(config.shortcut.switch_layout) {
        layout_state.layout = match layout_state.layout {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Vertical,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Grid,
        };
        reset_vix_evw.write(ResetVisibilityEvent);
        move_image_evw.write(MoveImageEvent);
    }
}

fn change_layout_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut reset_vix_evw: MessageWriter<ResetVisibilityEvent>,
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
        reset_vix_evw.write(ResetVisibilityEvent);
        move_image_evw.write(MoveImageEvent);
    }
}

fn change_top_image(
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
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

    move_image_evw.write(MoveImageEvent);
}

fn change_global_rotation(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut global_rotation: ResMut<GlobalRotation>,
) {
    if keys.just_pressed(config.shortcut.rotate_images) {
        global_rotation.0 += 1.;
    };
    move_image_evw.write(MoveImageEvent);
}

fn change_global_zoom(
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
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
        2.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit4) {
        3.
    } else if ctrl_pressed && keys.just_pressed(KeyCode::Digit5) {
        4.
    } else {
        return;
    };

    let zoom_factor = (2_f32).powf(scale_factor);
    global_scale.0 = zoom_factor;

    move_image_evw.write(MoveImageEvent);
}

fn change_rotation_individually(
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    mut sprite_query: Query<(&Id, &mut Rotation), With<MyImage>>,
) {
    if keys.pressed(config.shortcut.local_rotate_modifier) {
        if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
            return;
        }

        let num_images = sprite_query.iter().count();
        let window = windows.single().unwrap();

        let Some(cursor_position) = window.cursor_position() else {
            return;
        };

        let rotate_turn = if buttons.just_pressed(MouseButton::Left) {
            1.
        } else {
            -1.
        };

        for (id, mut rotate) in &mut sprite_query {
            let (cell_offset, cell_size) =
                get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);

            if cursor_position.x > cell_offset.x
                && cursor_position.x < cell_offset.x + cell_size.x
                && cursor_position.y > cell_offset.y
                && cursor_position.y < cell_offset.y + cell_size.y
            {
                rotate.0 += rotate_turn;
                break;
            }
        }

        move_image_evw.write(MoveImageEvent);
    }
}

fn change_zoom_individually(
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    mut sprite_query: Query<(&Id, &mut Scale, &mut Position), With<MyImage>>,
) {
    if keys.pressed(config.shortcut.local_zoom_modifier) {
        if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
            return;
        }

        let num_images = sprite_query.iter().count();
        let window = windows.single().unwrap();

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
                let (cell_offset, cell_size) =
                    get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);

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

        move_image_evw.write(MoveImageEvent);
    }
}

fn get_cell_rect(
    index: usize,
    num_images: usize,
    layout: &GridLayout,
    window: &Window,
    grid_width: i32,
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
            let grid_width = if grid_width == 0 {
                (num_images as f32).sqrt().ceil()
            } else {
                grid_width as f32
            };
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
        ui_state.settings_panel_visible = false;
    }
}

fn key_toggle_cursor(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    mut toggle_evw: MessageWriter<ToggleCursor>,
) {
    if keys.just_pressed(config.shortcut.switch_cursor) {
        toggle_evw.write(ToggleCursor);
    }
}

fn toggle_cursor(
    mut toggle_evr: MessageReader<ToggleCursor>,
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
                        Transform::from_translation(Vec3::new(0., 0., 1.)),
                        Visibility::default(),
                        Id(id.0),
                        MyCursor,
                    ))
                    .with_children(|parent| {
                        let cursor_color = Color::srgb(0.75, 0., 0.);
                        let bar_size = 15.;
                        let cursor_size = Some(Vec2::new(bar_size, 4.0));
                        parent.spawn((
                            Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            Transform::from_rotation(Quat::from_rotation_z(-TAU / 4.))
                                .with_translation(Vec3::new(bar_size, 0., 1.)),
                        ));
                        parent.spawn((
                            Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            Transform::from_rotation(Quat::from_rotation_z(-TAU / 4.))
                                .with_translation(Vec3::new(-bar_size, 0., 1.)),
                        ));
                        parent.spawn((
                            Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            Transform::from_translation(Vec3::new(0., bar_size, 1.)),
                        ));
                        parent.spawn((
                            Sprite {
                                color: cursor_color,
                                custom_size: cursor_size,
                                ..default()
                            },
                            Transform::from_translation(Vec3::new(0., -bar_size, 1.)),
                        ));
                    });
            }
        } else {
            *cursor_state = MultiCursorEnabled(false);
            for entity in &cursor_query {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn reset_scales(
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

fn on_image_spawned(
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
    mut reset_vis_evw: MessageWriter<ResetVisibilityEvent>,
    sprite_query: Query<&Id, Added<MyImage>>,
) {
    if sprite_query.iter().count() == 0 {
        return;
    }

    reset_vis_evw.write(ResetVisibilityEvent);
    fit_to_screen_evw.write(FitToScreen);
}

fn fit_to_screen(
    mut fit_to_screen_evr: MessageReader<FitToScreen>,
    windows: Query<&Window>,
    assets: Res<Assets<Image>>,
    mut global_scale: ResMut<GlobalScale>,
    mut sprite_query: Query<(&Id, &Sprite, &mut Scale, &mut Position), With<MyImage>>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
) {
    for _ev in fit_to_screen_evr.read() {
        let window = windows.single().unwrap();
        let num_images = sprite_query.iter().count();

        for (id, sprite, mut scale, mut position) in &mut sprite_query {
            let Some(image) = assets.get(&sprite.image) else {
                continue;
            };
            let image_size = image.size().as_vec2();

            let (_, cell_size) = get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);

            let factor = cell_size / image_size;

            if id.0 == 0 {
                global_scale.0 = f32::min(factor.x, factor.y);
                scale.0 = 1.0;
            } else {
                scale.0 = f32::min(factor.x, factor.y) / global_scale.0;
            }
            position.0 = Vec2::ZERO;
        }
    }
}

fn key_save_cropped(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    mut save_scropped_evw: MessageWriter<SaveCropped>,
) {
    if keys.just_pressed(config.shortcut.save_crop_image) {
        save_scropped_evw.write(SaveCropped);
    }
}

fn record_pressed_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut recorded_key: ResMut<RecordedPressedKey>,
    mut next_state: ResMut<NextState<MyAppState>>,
) {
    let mut count = false;
    for k in keys.get_pressed() {
        *recorded_key = RecordedPressedKey(Some(*k));
        count = true;
    }
    if !count && recorded_key.0.is_some() {
        // *recorded_key = RecordedPressedKey(None);
        next_state.set(MyAppState::Working);
    }
}

// insert a suffix to given filename
fn insert_suffix(path: &Path, suffix: &str) -> Option<std::path::PathBuf> {
    let parent = path.parent()?;
    let filename = path.file_stem()?.to_str()?;
    let extension = path.extension()?.to_str()?;
    Some(parent.join(filename.to_owned() + suffix + "." + extension))
}

fn save_cropped(
    mut save_scropped_evr: MessageReader<SaveCropped>,
    image_query: Query<(&ImagePath, &Sprite), With<MyImage>>,
) {
    for _ev in save_scropped_evr.read() {
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

            let mut reader = image::ImageReader::with_format(buf_in, format);

            // Remove the memory limit on image size we can read
            reader.no_limits();

            let Some(image) = reader.decode().ok() else {
                println!("Failed to decode image");
                continue;
            };
            // reader don't preserve the input format and append an alpha channel
            let image_rgb8 = image.to_rgb8();

            // Get Output buffer
            let Some(output_path) = insert_suffix(input_path, "_crop") else {
                println!("Failed to create output filename");
                continue;
            };

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
                &image_rgb8,
                rect.min.x as u32,
                rect.min.y as u32,
                size.x as u32,
                size.y as u32,
            );

            let subimage = image_view.to_image();

            // Save to disk
            if let Err(e) = subimage.write_to(&mut buf_out, ImageFormat::Jpeg) {
                println!("Failed to write data to file {}: {}", &output_path.display(), e);
                continue;
            };
        }
    }
}

fn save_settings(mut save_settings_evr: MessageReader<SaveSettingsEvent>, config: Res<Config>) {
    for _ev in save_settings_evr.read() {
        let Some(home_directory) = home::home_dir() else {
            println!("User directory not found");
            return;
        };
        println!("User Directory Found: {}", home_directory.display());
        let config_filename = ".image_viewer";
        println!("{}", home_directory.join(config_filename).display());
        let dst_path = home_directory.join(config_filename);

        let Ok(mut file) = File::create(dst_path) else {
            println!("Failed to create config file");
            return;
        };

        let Ok(config_str) = toml::to_string_pretty::<Config>(&*config) else {
            println!("Failed to serialize config");
            return;
        };
        let Ok(_) = file.write_all(config_str.as_bytes()) else {
            println!("Failed to write config to file");
            return;
        };
    }
}

fn scroll_events(
    config: Res<Config>,
    mut scroll_evr: MessageReader<MouseWheel>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut global_scale: ResMut<GlobalScale>,
    mut mouse_state: ResMut<MouseState>,
) {
    match config.misc.scroll_behavior {
        ScrollBehavior::Zoom => {
            use bevy::input::mouse::MouseScrollUnit;
            for ev in scroll_evr.read() {
                let scroll = match ev.unit {
                    MouseScrollUnit::Line => ev.y,
                    MouseScrollUnit::Pixel => ev.y,
                };

                let zoom_factor = if scroll > 0. { 1.1 } else { 0.9 };
                global_scale.0 *= zoom_factor;

                move_image_evw.write(MoveImageEvent);
            }
        }
        ScrollBehavior::Move => {
            use bevy::input::mouse::MouseScrollUnit;
            for ev in scroll_evr.read() {
                let scroll_vertical = match ev.unit {
                    MouseScrollUnit::Line => ev.y,
                    MouseScrollUnit::Pixel => ev.y,
                };
                let scroll_horizontal = match ev.unit {
                    MouseScrollUnit::Line => ev.x,
                    MouseScrollUnit::Pixel => ev.x,
                };

                mouse_state.delta += Vec2::new(scroll_horizontal, scroll_vertical);
                move_image_evw.write(MoveImageEvent);
            }
        }
        ScrollBehavior::None => {}
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

fn file_drop(
    mut dnd_evr: MessageReader<FileDragAndDrop>,
    mut is_new_batch: ResMut<NewImageBatch>,
    mut load_image_evw: MessageWriter<LoadNewImageEvent>,
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    sprite_query: Query<&Id, With<MyImage>>,
) {
    if dnd_evr.is_empty() {
        return;
    }
    let mut images_filename = Vec::new();

    let mut some_file_dropped = false;
    for ev in dnd_evr.read() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = ev {
            some_file_dropped = true;
            let Some(image_absolute) = path_buf.as_path().to_str() else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            images_filename.push(String::from(image_absolute));
        }
    }
    if some_file_dropped {
        let mut count: usize = sprite_query.iter().count();
        for (index, filename) in images_filename.into_iter().enumerate() {
            if !keys.pressed(config.shortcut.add_images) {
                is_new_batch.0 = true;
                count = 0;
            }
            load_image_evw.write(LoadNewImageEvent {
                path: filename,
                index: count + index,
            });
        }
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
