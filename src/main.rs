// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

mod review;

use std::f32::consts::{PI, TAU};
use std::fs::canonicalize;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
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

const HELP_STRING: &str = "Keyboard Shortcut:
    L: Change Layout (Grid, Stack, Horizontal, Vertical)
    Double-click: Switch between layout Grid-Stack or Horizontal-Vertical
    Shift + 1, 2, 3, ...: Move Image on top
    Ctrl/Cmd + 1, 2, 3, 4, 5: Zoom by 1, 2, 4, 8, 16
    Ctrl/Cmd + Shift + 1, 2, 3, 4, 5: Zoom by 1/2, 1/4, 1/8, 1/16, 1/32
    Z + Right/Left clic: zoom in/out the hovered image only
    R: Rotate all images CW
    E + Right/Left clic: rotate CW/CCW the hovered image only
    Q: Toggle 'Add Mode' (dropped images are added instead of replacing)
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

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
enum SamplerMode {
    Nearest,
    Bilinear,
}

// MARK: Config Struct
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
    sampler_mode: SamplerMode,
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

    let mut app = App::new();
    // add_plugins creates the winit EventLoop which registers the WinitApplicationDelegate class
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Image Viewer 3000".to_string(),
                    resolution: WindowResolution::new(1000, 350),
                    present_mode: PresentMode::AutoVsync,
                    ..default()
                }),
                ..default()
            })
            .set(ImagePlugin::default()),
        EguiPlugin::default(),
    ));

    // Inject the macOS dock drop handler before the event loop starts.
    // Must happen after add_plugins (class exists) but before run() (NSApp.run() called).
    // On cold launch, macOS calls application:openURLs: before applicationDidFinishLaunching:,
    // so the method must be on the class before the Cocoa event loop begins.
    #[cfg(target_os = "macos")]
    macos_dock_drop::inject_open_urls_handler();

    app.insert_state(MyAppState::Working)
        .insert_resource(InitialImagesFilename(images_filename))
        .insert_resource(UiState {
            visible: true,
            settings_panel_visible: false,
            image_list_visible: false,
        })
        .insert_resource(config_data)
        .insert_resource(GlobalScale(1. / 8.))
        .insert_resource(GlobalRotation(0))
        .insert_resource(NewImageBatch(true))
        .insert_resource(AddMode(false))
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
        .insert_resource(ImageOrder(Vec::new()))
        .insert_resource(ReviewState::default())
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
        .add_message::<ChangeSamplerEvent>()
        .add_message::<RemoveImageEvent>()
        .add_message::<ReorderImagesEvent>()
        .add_message::<NavigateReviewEvent>()
        .add_message::<RefreshReviewEvent>()
        .add_message::<ActivateReviewEvent>()
        // Egui systems must run in EguiPrimaryContextPass (not Update)
        .add_systems(EguiPrimaryContextPass, configure_visuals.run_if(run_once))
        .add_systems(
            EguiPrimaryContextPass,
            (
                ui_bottom_menu,
                ui_image_list_panel.after(ui_bottom_menu),
                ui_settings_menu.after(ui_bottom_menu),
                ui_review_panel.after(ui_bottom_menu),
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
                key_toggle_add_mode,
                save_cropped,
                save_settings,
                change_image_title_style,
                change_sampler,
                on_remove_image,
                on_reorder_images,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(
            Update,
            (
                on_navigate_review,
                on_activate_review,
                on_refresh_review,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(Update, record_pressed_key.run_if(in_state(MyAppState::EditShortCut)))
        .add_systems(Update, poll_dock_drop_queue.run_if(in_state(MyAppState::Working)))
        .run();

    Ok(())
}

// MARK: State Struct
#[derive(Debug, Resource)]
struct MultiCursorEnabled(bool);

#[derive(Resource)]
struct UiState {
    visible: bool,
    settings_panel_visible: bool,
    image_list_visible: bool,
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
struct GlobalRotation(i32);

#[derive(Resource)]
struct NewImageBatch(bool);

#[derive(Resource)]
struct AddMode(bool);

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

// Maps display slot -> image Id. Allows reordering images in the list panel.
#[derive(Resource)]
struct ImageOrder(Vec<usize>);

#[derive(Resource, Default)]
struct ReviewState {
    enabled: bool,
    directory: String,
    cell_patterns: Vec<review::CellPattern>,
    radixes: Vec<String>,
    current_index: usize,
    editable_patterns: Vec<String>,
    error: Option<String>,
}

// MARK: Components
#[derive(Component)]
struct Id(usize);

#[derive(Component)]
struct Scale(f32);

#[derive(Component)]
struct Position(Vec2);

// Rotation in quarter turn (1 is 1 turn)
#[derive(Component)]
struct Rotation(i32);

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

// MARK: Messages
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
struct ChangeSamplerEvent;

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

#[derive(Message)]
struct RemoveImageEvent(usize); // the Id of the image to remove

#[derive(Message)]
struct ReorderImagesEvent; // signal to recompute layout after reorder

#[derive(Message)]
struct NavigateReviewEvent(i32); // +1 next, -1 previous

#[derive(Message)]
struct RefreshReviewEvent;

#[derive(Message)]
struct ActivateReviewEvent;

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
    mut add_mode: ResMut<AddMode>,
    mut review_state: ResMut<ReviewState>,
    mut activate_evw: MessageWriter<ActivateReviewEvent>,
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
                    ui.toggle_value(&mut ui_state.settings_panel_visible, "\u{2699}");
                    ui.toggle_value(&mut ui_state.image_list_visible, "\u{2630}");
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

                    ui.separator();
                    ui.toggle_value(&mut add_mode.0, "Add")
                        .on_hover_text("When enabled, dropped images are added instead of replacing");

                    ui.separator();
                    ui.toggle_value(&mut review_state.enabled, "Review")
                        .on_hover_text("Review mode: navigate through similar images");
                    if review_state.enabled && review_state.radixes.is_empty() {
                        activate_evw.write(ActivateReviewEvent);
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
    mut change_sampler_evw: MessageWriter<ChangeSamplerEvent>,
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

                ui.horizontal(|ui| {
                    ui.label("Interpolation:");
                    egui::ComboBox::from_label(" ")
                        .selected_text(format!("{:?}", config.misc.sampler_mode))
                        .show_ui(ui, |ui| {
                            let r1 = ui
                                .selectable_value(&mut config.misc.sampler_mode, SamplerMode::Nearest, "Nearest")
                                .changed();
                            let r2 = ui
                                .selectable_value(&mut config.misc.sampler_mode, SamplerMode::Bilinear, "Bilinear")
                                .changed();
                            if r1 || r2 {
                                change_sampler_evw.write(ChangeSamplerEvent);
                            }
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

fn ui_image_list_panel(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut image_order: ResMut<ImageOrder>,
    image_path_query: Query<(&Id, &ImagePath), With<MyImage>>,
    mut remove_image_evw: MessageWriter<RemoveImageEvent>,
    mut reorder_evw: MessageWriter<ReorderImagesEvent>,
) {
    if !ui_state.image_list_visible {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    // Build a lookup from image Id -> short name
    let mut name_map: Vec<(usize, String)> = Vec::new();
    for (id, path) in &image_path_query {
        let short = get_short_name(&path.0).unwrap_or("?");
        name_map.push((id.0, short.to_string()));
    }

    egui::SidePanel::left("Image List")
        .resizable(true)
        .default_width(180.)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Images");
            });
            ui.separator();

            let mut to_remove: Option<usize> = None;
            let mut from_slot: Option<usize> = None;
            let mut to_slot: Option<usize> = None;

            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                // We iterate in display order
                for (slot, &image_id) in image_order.0.iter().enumerate() {
                    let name = name_map
                        .iter()
                        .find(|(id, _)| *id == image_id)
                        .map(|(_, n)| n.as_str())
                        .unwrap_or("?");

                    let row_id = egui::Id::new("image_list_row").with(slot);

                    // Row layout: [drag-source label] [remove button]
                    // The remove button is outside the drag source so it receives clicks properly.
                    ui.horizontal(|ui| {
                        let response = ui
                            .dnd_drag_source(row_id, slot, |ui| {
                                ui.label(format!("{}. {}", slot + 1, name));
                            })
                            .response;

                        if ui.button("\u{2716}").on_hover_text("Remove image").clicked() {
                            to_remove = Some(image_id);
                        }

                        // Show a drop indicator line and determine insertion point
                        if let (Some(pointer), Some(hovered_payload)) = (
                            ui.input(|i| i.pointer.interact_pos()),
                            response.dnd_hover_payload::<usize>(),
                        ) {
                            let rect = response.rect;
                            let stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);

                            let insert_slot = if *hovered_payload == slot {
                                ui.painter().hline(rect.x_range(), rect.center().y, stroke);
                                slot
                            } else if pointer.y < rect.center().y {
                                ui.painter().hline(rect.x_range(), rect.top(), stroke);
                                slot
                            } else {
                                ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
                                slot + 1
                            };

                            if let Some(dragged_payload) = response.dnd_release_payload::<usize>() {
                                from_slot = Some(*dragged_payload);
                                to_slot = Some(insert_slot);
                            }
                        }
                    });
                }
            });

            // Handle reorder (adjust index when moving within the same list)
            if let (Some(src), Some(mut dst)) = (from_slot, to_slot) {
                dst -= (src < dst) as usize;
                if src != dst {
                    let item = image_order.0.remove(src);
                    image_order.0.insert(dst, item);
                    reorder_evw.write(ReorderImagesEvent);
                }
            }

            // Handle removal
            if let Some(id_to_remove) = to_remove {
                remove_image_evw.write(RemoveImageEvent(id_to_remove));
            }
        });
}

fn on_remove_image(
    mut remove_evr: MessageReader<RemoveImageEvent>,
    mut commands: Commands,
    mut query_by_id: Query<(Entity, &mut Id)>,
    mut image_order: ResMut<ImageOrder>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
) {
    for ev in remove_evr.read() {
        let target_id = ev.0;

        // Despawn all entity with matching Id
        for (entity, id) in &query_by_id {
            if id.0 == target_id {
                commands.entity(entity).despawn();
                continue;
            }
        }

        // Remove from ImageOrder, keeping remaining in display order
        image_order.0.retain(|&id| id != target_id);

        // Reassign contiguous Ids on remaining entities using two passes to avoid collisions.
        let old_ids: Vec<usize> = image_order.0.clone();
        let count = old_ids.len();

        // Pass 1: shift to temporary range
        for &old_id in &old_ids {
            let temp_id = old_id + count + 1;
            for (_, mut id) in &mut query_by_id {
                if id.0 == old_id {
                    id.0 = temp_id;
                    continue;
                }
            }
        }

        // Pass 2: assign final contiguous values
        for (new_idx, &old_id) in old_ids.iter().enumerate() {
            let temp_id = old_id + count + 1;
            for (_, mut id) in &mut query_by_id {
                if id.0 == temp_id {
                    id.0 = new_idx;
                    continue;
                }
            }
        }

        // Rebuild ImageOrder as contiguous [0, 1, 2, ...]
        let count = image_order.0.len();
        image_order.0 = (0..count).collect();

        fit_to_screen_evw.write(FitToScreen);
        move_image_evw.write(MoveImageEvent);
    }
}

fn on_reorder_images(
    mut reorder_evr: MessageReader<ReorderImagesEvent>,
    mut image_order: ResMut<ImageOrder>,
    mut query_by_id: Query<&mut Id>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut reset_vis_evw: MessageWriter<ResetVisibilityEvent>,
) {
    if reorder_evr.is_empty() {
        return;
    }
    reorder_evr.clear();

    // After a drag-and-drop reorder, ImageOrder[new_slot] = old_id.
    // We need to reassign Id components so that each entity's Id matches its new display slot.
    // A direct loop fails when two slots swap because the first rename collides with the second.
    // Fix: two passes — first shift all Ids to a temporary range, then assign final values.
    let order_snapshot: Vec<usize> = image_order.0.clone();
    let count = order_snapshot.len();

    // Pass 1: rename old_id -> old_id + count (temporary, guaranteed unique)
    for &old_id in &order_snapshot {
        let temp_id = old_id + count;
        for mut id in &mut query_by_id {
            if id.0 == old_id {
                id.0 = temp_id;
                continue;
            }
        }
    }

    // Pass 2: rename temp_id -> new_idx (final contiguous value)
    for (new_idx, &old_id) in order_snapshot.iter().enumerate() {
        let temp_id = old_id + count;
        for mut id in &mut query_by_id {
            if id.0 == temp_id {
                id.0 = new_idx;
                continue;
            }
        }
    }

    // Rebuild ImageOrder as contiguous [0, 1, 2, ...]
    image_order.0 = (0..count).collect();

    move_image_evw.write(MoveImageEvent);
    reset_vis_evw.write(ResetVisibilityEvent);
}

fn on_load_image(
    mut load_evr: MessageReader<LoadNewImageEvent>,
    mut loaded_evw: MessageWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
    config: Res<Config>,
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
        // This is required to process large images that would otherwise be rejected by the image crate
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
        let mut loaded_image = loaded_image;
        loaded_image.sampler = match config.misc.sampler_mode {
            SamplerMode::Nearest => ImageSampler::Descriptor(ImageSamplerDescriptor::nearest()),
            SamplerMode::Bilinear => ImageSampler::Descriptor(ImageSamplerDescriptor::linear()),
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
    mut is_new_batch: ResMut<NewImageBatch>,
    mut image_order: ResMut<ImageOrder>,
) {
    for ev in load_image_evr.read() {
        let font = font_query.single().unwrap();

        if is_new_batch.0 {
            for entity in &images {
                commands.entity(entity).despawn();
            }
            image_order.0.clear();
            is_new_batch.0 = false;
        }

        // Start hidden; on_move_image will make it visible after positioning
        commands.spawn((
            Sprite {
                image: ev.handle.clone(),
                ..default()
            },
            Visibility::Hidden,
            Id(ev.index),
            Scale(1.),
            Position(Vec2::ZERO),
            Rotation(0),
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
            Visibility::Hidden,
            Id(ev.index),
            MyText,
        ));

        // Track the new image in display order
        image_order.0.push(ev.index);

        let mut help_visibility = help_query.single_mut().unwrap();
        *help_visibility = Visibility::Hidden;
    }
}

fn on_move_image(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    assets: Res<Assets<Image>>,
    mut sprite_position: Query<
        (&Id, &Position, &Scale, &Rotation, &mut Transform, &mut Sprite, &mut Visibility),
        With<MyImage>,
    >,
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
    for (id, position, scale, rotation, mut transform, mut sprite, mut visibility) in &mut sprite_position {
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

        sprite.rect = Some(cell.intersect(image_crop));

        // Make visible after positioning (sprites start hidden to avoid flash at native resolution)
        *visibility = match layout_state.layout {
            GridLayout::Stack => {
                if id.0 == layout_state.index {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                }
            }
            _ => Visibility::Visible,
        };
    }

    let (_, cell_size) = get_cell_rect(0, num_images, &layout_state.layout, window, config.misc.grid_width);
    for mut node in &mut title_query {
        node.width = Val::Px(cell_size.x);
    }
}

fn on_move_image_title(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&Id, &mut Node, &mut Visibility), With<MyText>>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let num_images = text_query.iter().count();
    let window = windows.single().unwrap();

    for (id, mut node, mut visibility) in &mut text_query {
        let (cell_offset, _) = get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);
        node.top = Val::Px(cell_offset.y + 2.);
        node.left = Val::Px(cell_offset.x + 5.);
        *visibility = match layout_state.layout {
            GridLayout::Stack => {
                if id.0 == layout_state.index {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                }
            }
            _ => Visibility::Visible,
        };
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
        global_rotation.0 += 1;
    move_image_evw.write(MoveImageEvent);
    };
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
            1
        } else {
            -1
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
    sprite_query: Query<&Id, Added<MyImage>>,
) {
    if sprite_query.iter().count() == 0 {
        return;
    }

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
    mut move_image_evw: MessageWriter<MoveImageEvent>,
) {
    for _ev in fit_to_screen_evr.read() {
        let window = windows.single().unwrap();
        let num_images = sprite_query.iter().count();

        let mut first = true;
        for (id, sprite, mut scale, mut position) in &mut sprite_query {
            let Some(image) = assets.get(&sprite.image) else {
                continue;
            };
            let image_size = image.size().as_vec2();
            let (_, cell_size) = get_cell_rect(id.0, num_images, &layout_state.layout, window, config.misc.grid_width);
            let factor = f32::min(cell_size.x / image_size.x, cell_size.y / image_size.y);

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

fn key_save_cropped(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<Config>,
    mut save_cropped_evw: MessageWriter<SaveCropped>,
) {
    if keys.just_pressed(config.shortcut.save_crop_image) {
        save_cropped_evw.write(SaveCropped);
    }
}

fn key_toggle_add_mode(keys: Res<ButtonInput<KeyCode>>, config: Res<Config>, mut add_mode: ResMut<AddMode>) {
    if keys.just_pressed(config.shortcut.add_images) {
        add_mode.0 = !add_mode.0;
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
    mut save_cropped_evr: MessageReader<SaveCropped>,
    image_query: Query<(&ImagePath, &Sprite), With<MyImage>>,
) {
    for _ev in save_cropped_evr.read() {
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
            // This is required to process large images that would otherwise be rejected by the image crate
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

fn change_sampler(
    mut change_sampler_evr: MessageReader<ChangeSamplerEvent>,
    config: Res<Config>,
    sprite_query: Query<&Sprite, With<MyImage>>,
    mut images: ResMut<Assets<Image>>,
) {
    if change_sampler_evr.is_empty() {
        return;
    }
    change_sampler_evr.clear();

    let new_sampler = match config.misc.sampler_mode {
        SamplerMode::Nearest => ImageSampler::Descriptor(ImageSamplerDescriptor::nearest()),
        SamplerMode::Bilinear => ImageSampler::Descriptor(ImageSamplerDescriptor::linear()),
    };

    for sprite in &sprite_query {
        let Some(image) = images.get_mut(&sprite.image) else {
            continue;
        };
        image.sampler = new_sampler.clone();
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
    add_mode: Res<AddMode>,
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
        let mut count: usize = if add_mode.0 {
            sprite_query.iter().count()
        } else {
            is_new_batch.0 = true;
            0
        };
        for filename in images_filename {
            load_image_evw.write(LoadNewImageEvent {
                path: filename,
                index: count,
            });
            count += 1;
        }
    }
}

fn bound(vec: Vec2, rect: Rect) -> Vec2 {
    Vec2::new(vec.x.clamp(rect.min.x, rect.max.x), vec.y.clamp(rect.min.y, rect.max.y))
}

// MARK: macOS Dock Drop
// macOS requires the NSApplicationDelegate to implement application:openURLs: for dock icon
// drops and "Open With" to work. Winit 0.30.x doesn't implement this, so we inject the method
// into WinitApplicationDelegate at runtime. When winit 0.31+ lands (which exposes delegate
// registration), this workaround can be removed.

static DOCK_DROP_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(target_os = "macos")]
mod macos_dock_drop {
    use objc2::ffi;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;
    use objc2_foundation::{NSArray, NSURL};

    // The handler that the ObjC runtime will call for application:openURLs:
    // Signature: void(id self, SEL _cmd, id application, id urls)
    unsafe extern "C" fn handle_open_urls(_this: &AnyObject, _cmd: Sel, _sender: &AnyObject, urls: &NSArray<NSURL>) {
        let mut paths = Vec::new();
        for i in 0..urls.len() {
            let Some(url) = urls.get(i) else { continue };
            let Some(ns_path) = (unsafe { url.path() }) else {
                continue;
            };
            let path = ns_path.to_string();
            println!("macOS dock drop: {}", path);
            paths.push(path);
        }

        if !paths.is_empty() {
            let Ok(mut queue) = super::DOCK_DROP_QUEUE.lock() else {
                return;
            };
            queue.extend(paths);
        }
    }

    // Inject application:openURLs: into WinitApplicationDelegate at runtime.
    // Must be called after the event loop is created (so the class is registered).
    pub fn inject_open_urls_handler() {
        let Some(cls) = AnyClass::get("WinitApplicationDelegate") else {
            println!("macOS dock drop: WinitApplicationDelegate class not found, skipping");
            return;
        };

        let sel = sel!(application:openURLs:);
        // Type encoding: void(id, SEL, id, id) = "v@:@@"
        let types = c"v@:@@";
        let imp: Imp = unsafe { std::mem::transmute(handle_open_urls as *const ()) };
        let cls_ptr = cls as *const AnyClass as *mut ffi::objc_class;

        // SAFETY: cls_ptr points to a valid, registered ObjC class. The selector, type encoding,
        // and function signature all match the application:openURLs: delegate method.
        let ok = unsafe { ffi::class_addMethod(cls_ptr, sel.as_ptr(), Some(imp), types.as_ptr()) };
        if ok {
            println!("macOS dock drop: injected application:openURLs: handler");
        } else {
            println!("macOS dock drop: failed to inject handler (method may already exist)");
        }
    }
}

fn poll_dock_drop_queue(
    mut is_new_batch: ResMut<NewImageBatch>,
    mut load_image_evw: MessageWriter<LoadNewImageEvent>,
    add_mode: Res<AddMode>,
    sprite_query: Query<&Id, With<MyImage>>,
) {
    let Ok(mut queue) = DOCK_DROP_QUEUE.lock() else {
        return;
    };
    if queue.is_empty() {
        return;
    }

    let paths: Vec<String> = queue.drain(..).collect();
    drop(queue);

    let count: usize = if add_mode.0 { sprite_query.iter().count() } else { 0 };
    if !add_mode.0 {
        is_new_batch.0 = true;
    }
    for (index, path) in paths.into_iter().enumerate() {
        load_image_evw.write(LoadNewImageEvent {
            path,
            index: count + index,
        });
    }
}

// MARK: Review Mode

fn on_navigate_review(
    mut navigate_evr: MessageReader<NavigateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    mut is_new_batch: ResMut<NewImageBatch>,
    mut load_image_evw: MessageWriter<LoadNewImageEvent>,
) {
    for ev in navigate_evr.read() {
        if review_state.radixes.is_empty() {
            continue;
        }
        let count = review_state.radixes.len() as i32;
        let new_index = (review_state.current_index as i32 + ev.0).rem_euclid(count) as usize;
        review_state.current_index = new_index;

        let radix = &review_state.radixes[new_index];
        let directory = PathBuf::from(&review_state.directory);
        let files = review::resolve_files_for_radix(&directory, radix, &review_state.cell_patterns);

        is_new_batch.0 = true;
        for (index, file) in files.into_iter().enumerate() {
            let Some(path) = file else { continue };
            load_image_evw.write(LoadNewImageEvent { path, index });
        }
    }
}

fn on_activate_review(
    mut activate_evr: MessageReader<ActivateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    image_query: Query<&ImagePath, With<MyImage>>,
) {
    for _ev in activate_evr.read() {
        review_state.error = None;
        let paths: Vec<String> = image_query.iter().map(|p| p.0.clone()).collect();
        if paths.len() < 2 {
            review_state.error = Some("Review mode needs at least 2 images".to_string());
            continue;
        }

        // Extract directory from the first image path
        let Some(dir) = Path::new(&paths[0]).parent() else {
            review_state.error = Some("Cannot determine directory from image path".to_string());
            continue;
        };

        // Get just the filenames for pattern extraction
        let filenames: Vec<String> = paths
            .iter()
            .filter_map(|p| Path::new(p).file_name().and_then(|f| f.to_str()).map(String::from))
            .collect();
        let filename_refs: Vec<&str> = filenames.iter().map(|s| s.as_str()).collect();

        let Some(result) = review::extract_patterns(&filename_refs) else {
            review_state.error = Some("No common pattern found in filenames".to_string());
            continue;
        };

        let radixes = review::scan_radixes(dir, &result.cell_patterns);
        let current_index = radixes.iter().position(|r| r == &result.radix).unwrap_or(0);

        review_state.enabled = true;
        review_state.directory = dir.to_string_lossy().to_string();
        review_state.editable_patterns = result.cell_patterns.iter().map(|cp| cp.regex_str.clone()).collect();
        review_state.cell_patterns = result.cell_patterns;
        review_state.radixes = radixes;
        review_state.current_index = current_index;
    }
}

fn on_refresh_review(
    mut refresh_evr: MessageReader<RefreshReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    mut is_new_batch: ResMut<NewImageBatch>,
    mut load_image_evw: MessageWriter<LoadNewImageEvent>,
) {
    for _ev in refresh_evr.read() {
        // Rebuild cell_patterns from editable_patterns
        let mut new_patterns = Vec::new();
        for (i, regex_str) in review_state.editable_patterns.iter().enumerate() {
            let old = review_state.cell_patterns.get(i);
            let tail = old.map(|cp| cp.tail.clone()).unwrap_or_default();
            new_patterns.push(review::CellPattern {
                tail,
                regex_str: regex_str.clone(),
            });
        }
        review_state.cell_patterns = new_patterns;

        let directory = PathBuf::from(&review_state.directory);
        review_state.radixes = review::scan_radixes(&directory, &review_state.cell_patterns);
        if review_state.current_index >= review_state.radixes.len() {
            review_state.current_index = 0;
        }

        // Reload images for the current radix
        if let Some(radix) = review_state.radixes.get(review_state.current_index) {
            let files = review::resolve_files_for_radix(&directory, radix, &review_state.cell_patterns);
            is_new_batch.0 = true;
            for (index, file) in files.into_iter().enumerate() {
                let Some(path) = file else { continue };
                load_image_evw.write(LoadNewImageEvent { path, index });
            }
        }
    }
}

fn ui_review_panel(
    mut contexts: EguiContexts,
    mut review_state: ResMut<ReviewState>,
    mut navigate_evw: MessageWriter<NavigateReviewEvent>,
    mut refresh_evw: MessageWriter<RefreshReviewEvent>,
    mut activate_evw: MessageWriter<ActivateReviewEvent>,
    ui_state: Res<UiState>,
) {
    if !ui_state.visible || !review_state.enabled {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::TopBottomPanel::bottom("review_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if let Some(error) = &review_state.error {
                ui.colored_label(egui::Color32::from_rgb(255, 150, 100), error.as_str());
            } else {
                if ui.button("\u{25C0}").clicked() {
                    navigate_evw.write(NavigateReviewEvent(-1));
                }
                if ui.button("\u{25B6}").clicked() {
                    navigate_evw.write(NavigateReviewEvent(1));
                }

                let total = review_state.radixes.len();
                let current = review_state.current_index;
                let radix_name = review_state.radixes.get(current).cloned().unwrap_or_default();
                ui.label(format!("{}/{}: {}", current + 1, total, radix_name));

                ui.separator();

                for pattern in review_state.editable_patterns.iter_mut() {
                    ui.add(egui::TextEdit::singleline(pattern).desired_width(200.0));
                    ui.separator();
                }

                if ui.button("\u{21BB}").on_hover_text("Reload directory with current regexes").clicked() {
                    refresh_evw.write(RefreshReviewEvent);
                }
            }

            if ui.button("\u{2672}").on_hover_text("Recompute patterns from open images").clicked() {
                activate_evw.write(ActivateReviewEvent);
            }
        });
    });
}

fn check_all_images_exist(images: &[String]) -> Result<Vec<String>> {
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

fn get_short_name(path: &str) -> Option<&str> {
    Path::new(path).file_name()?.to_str()
}
