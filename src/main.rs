#![windows_subsystem = "windows"]
// Bevy systems take their resources and queries as parameters, so long signatures are normal
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod color;
mod dock_drop;
mod input;
mod layout;
mod loading;
mod review;
mod review_mode;
mod ui;

use std::env::home_dir;
use std::fs::canonicalize;
use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

use bevy::camera::Hdr;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::window::{PresentMode, RequestRedraw, WindowResolution};
use bevy::winit::WinitSettings;
use bevy_egui::{egui, EguiPlugin, EguiPrimaryContextPass};
use clap::Parser;
use moxcms::ColorProfile;
use serde::{Deserialize, Serialize};

use crate::dock_drop::{poll_dock_drop_queue, store_event_loop_proxy};
use crate::input::{
    change_global_rotation, change_global_zoom, change_layout_on_click, change_rotation_individually, change_top_image,
    change_zoom_individually, cursor_move, file_drop, key_change_layout, key_save_cropped, key_toggle_add_mode,
    key_toggle_cursor, mouse_button_input, record_pressed_key, scroll_events, toggle_help,
};
use crate::layout::{
    change_image_title_style, fit_to_screen, on_image_spawned, on_move_cursor, on_move_image, on_move_image_title,
    on_resize_system, reset_scales, sync_cursors,
};
use crate::loading::{change_sampler, on_image_loaded, on_load_image, on_reload_images, on_remove_image, save_cropped};
use crate::review_mode::{on_activate_review, on_navigate_review, on_refresh_review, on_switch_review_to_file};
use crate::ui::{
    configure_visuals, ui_bottom_menu, ui_edit_short_cut, ui_image_list_panel, ui_review_panel, ui_settings_menu,
};

#[doc(hidden)]
type Result<T> = ::std::result::Result<T, Box<dyn ::std::error::Error>>;

#[derive(Parser, Debug)]
#[clap(version, long_about = None)]
struct Args {
    // Images to show
    images: Vec<String>,
}

// Largest 2D texture side assumed when the render device isn't available yet (most Apple GPUs on Metal).
// Otherwise the device limit is used. Wider or taller images are split into tiles.
const FALLBACK_MAX_TEXTURE_SIZE: u32 = 16384;

const CTRL_KEYS: [KeyCode; 2] = [KeyCode::ControlLeft, KeyCode::ControlRight];
const SHIFT_KEYS: [KeyCode; 2] = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
// In keyboard order, so a key's index is its digit minus one (0 comes last)
const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

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

impl SamplerMode {
    fn sampler(&self) -> ImageSampler {
        match self {
            SamplerMode::Nearest => ImageSampler::Descriptor(ImageSamplerDescriptor::nearest()),
            SamplerMode::Bilinear => ImageSampler::Descriptor(ImageSamplerDescriptor::linear()),
        }
    }
}

// Color space the physical screen works in. Decoded images are converted from
// their own color space (embedded ICC profile, or assumed sRGB) to this one,
// so a wide-gamut monitor can be fed wide-gamut values. The rendered values
// are sent to the screen as-is on every platform: it is up to the user to
// select the space matching their monitor.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy, Default)]
enum ScreenColorSpace {
    #[default]
    Srgb,
    AdobeRgb,
    DisplayP3,
    Rec2020,
}

impl ScreenColorSpace {
    const ALL: [ScreenColorSpace; 4] = [
        ScreenColorSpace::Srgb,
        ScreenColorSpace::AdobeRgb,
        ScreenColorSpace::DisplayP3,
        ScreenColorSpace::Rec2020,
    ];

    fn profile(&self) -> ColorProfile {
        match self {
            ScreenColorSpace::Srgb => ColorProfile::new_srgb(),
            ScreenColorSpace::AdobeRgb => ColorProfile::new_adobe_rgb(),
            ScreenColorSpace::DisplayP3 => ColorProfile::new_display_p3(),
            ScreenColorSpace::Rec2020 => ColorProfile::new_bt2020(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ScreenColorSpace::Srgb => "sRGB",
            ScreenColorSpace::AdobeRgb => "Adobe RGB",
            ScreenColorSpace::DisplayP3 => "Display P3",
            ScreenColorSpace::Rec2020 => "Rec. 2020",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct ConfigColor {
    screen_color_space: ScreenColorSpace,
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

impl ConfigShortcut {
    // Labelled shortcuts, in the order shown in the settings panel
    fn entries_mut(&mut self) -> [(&'static str, &mut KeyCode); 7] {
        [
            ("Save Crop:", &mut self.save_crop_image),
            ("Local Zoom Modifier:", &mut self.local_zoom_modifier),
            ("Local Rotate Modifier:", &mut self.local_rotate_modifier),
            ("Change Cursor:", &mut self.switch_cursor),
            ("Rotate:", &mut self.rotate_images),
            ("Add Images:", &mut self.add_images),
            ("Change Layout:", &mut self.switch_layout),
        ]
    }
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
    // default so config files predating this section keep loading
    #[serde(default)]
    color: ConfigColor,
}

// MARK: Main
fn main() -> Result<()> {
    let args = Args::parse();
    let images_filename = check_all_images_exist(&args.images)?;

    let user_config_data = 'block: {
        let Some(config_path) = config_path() else {
            break 'block None;
        };
        let Some(config_str) = std::fs::read_to_string(&config_path).ok() else {
            println!("Config File not found: {}", config_path.display());
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
                return Err("Invalid default config: the embedded assets/default/config.toml doesn't parse".into());
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
    dock_drop::macos_dock_drop::inject_open_urls_handler();

    app.insert_state(MyAppState::Working)
        // Only update on input (or when something asks for a redraw) instead of spinning every frame.
        // Without this the app burns CPU while idle, and even more when minimized since vsync no longer paces it.
        .insert_resource(WinitSettings::desktop_app())
        .insert_resource(InitialImagesFilename(images_filename))
        .insert_resource(UiState {
            visible: true,
            settings_panel_visible: false,
            image_list_visible: false,
        })
        .insert_resource(config_data)
        .insert_resource(GlobalScale(1. / 8.))
        .insert_resource(GlobalRotation(0))
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
        .init_resource::<ImageList>()
        .init_resource::<UiPanelArea>()
        .insert_resource(ReviewState::default())
        .insert_resource(PreservedView::default())
        .add_systems(Startup, setup)
        .add_message::<LoadImagesEvent>()
        .add_message::<NewImageLoadedEvent>()
        .add_message::<MoveImageEvent>()
        .add_message::<SaveCropped>()
        .add_message::<ResetScales>()
        .add_message::<FitToScreen>()
        .add_message::<ChangeTitleStyleEvent>()
        .add_message::<SaveSettingsEvent>()
        .add_message::<ChangeSamplerEvent>()
        .add_message::<RemoveImageEvent>()
        .add_message::<ReloadImagesEvent>()
        .add_message::<NavigateReviewEvent>()
        .add_message::<RefreshReviewEvent>()
        .add_message::<ActivateReviewEvent>()
        .add_message::<SwitchReviewToFileEvent>()
        // Egui systems must run in EguiPrimaryContextPass (not Update)
        .add_systems(EguiPrimaryContextPass, configure_visuals.run_if(run_once))
        .add_systems(
            EguiPrimaryContextPass,
            (
                ui_bottom_menu,
                ui_image_list_panel.after(ui_bottom_menu),
                ui_settings_menu.after(ui_image_list_panel),
                ui_review_panel.after(ui_settings_menu),
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
                on_resize_system,
                on_image_loaded,
                on_move_cursor,
                on_move_image.after(on_resize_system),
                on_move_image_title.after(on_resize_system),
                on_load_image,
                on_image_spawned,
                toggle_help,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        // Bevy doesn't allow more than 20 systems in an anonymous system set, hence several add_systems
        .add_systems(
            Update,
            (
                change_top_image,
                change_global_rotation,
                key_toggle_cursor,
                sync_cursors,
                reset_scales,
                fit_to_screen,
                key_save_cropped,
                key_toggle_add_mode,
                save_cropped,
                save_settings,
                change_image_title_style,
                change_sampler,
                on_remove_image,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(
            Update,
            (
                on_navigate_review,
                on_activate_review,
                on_refresh_review,
                on_switch_review_to_file,
                on_reload_images,
            )
                .run_if(in_state(MyAppState::Working)),
        )
        .add_systems(Update, record_pressed_key.run_if(in_state(MyAppState::EditShortCut)))
        .add_systems(Update, poll_dock_drop_queue.run_if(in_state(MyAppState::Working)))
        .add_systems(Startup, store_event_loop_proxy)
        // In reactive mode a message written this frame is only read next frame, which would never come
        // without new input. Keep updating while any message chain (or a texture upload) is still in flight.
        .add_systems(
            Last,
            (
                redraw_while_pending::<LoadImagesEvent>,
                redraw_while_pending::<NewImageLoadedEvent>,
                redraw_while_pending::<MoveImageEvent>,
                redraw_while_pending::<SaveCropped>,
                redraw_while_pending::<ResetScales>,
                redraw_while_pending::<FitToScreen>,
                redraw_while_pending::<ChangeTitleStyleEvent>,
                redraw_while_pending::<SaveSettingsEvent>,
                redraw_while_pending::<ChangeSamplerEvent>,
                redraw_while_pending::<RemoveImageEvent>,
                redraw_while_pending::<ReloadImagesEvent>,
                redraw_while_pending::<NavigateReviewEvent>,
                redraw_while_pending::<RefreshReviewEvent>,
                redraw_while_pending::<ActivateReviewEvent>,
                redraw_while_pending::<SwitchReviewToFileEvent>,
                redraw_while_pending::<AssetEvent<Image>>,
            ),
        )
        .run();

    Ok(())
}

// MARK: State Struct
#[derive(Debug, Resource)]
struct MultiCursorEnabled(bool);

// Screen area not yet consumed by egui panels. Panels are drawn inside a root
// Ui and stack by shrinking its available rect, but our panels are spread over
// several bevy systems that each build their own root Ui, so the leftover area
// must be carried from one system to the next. Always go through
// PanelRootUi::new(): the guard writes the leftover area back on drop, and the
// pass number resets the area to the full viewport once per egui pass.
#[derive(Resource, Default)]
struct UiPanelArea {
    pass_nr: u64,
    rect: Option<egui::Rect>,
}

#[derive(Resource)]
struct UiState {
    visible: bool,
    settings_panel_visible: bool,
    image_list_visible: bool,
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum GridLayout {
    Stack,
    Horizontal,
    Vertical,
    Grid,
}

impl GridLayout {
    // In the order of the layout shortcut cycle
    const ALL: [GridLayout; 4] = [
        GridLayout::Grid,
        GridLayout::Stack,
        GridLayout::Vertical,
        GridLayout::Horizontal,
    ];

    fn label(&self) -> &'static str {
        match self {
            GridLayout::Grid => "Grid",
            GridLayout::Stack => "Stack",
            GridLayout::Vertical => "Vertical",
            GridLayout::Horizontal => "Horizontal",
        }
    }

    fn next(&self) -> Self {
        match self {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Vertical,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Grid,
        }
    }

    // Double-click switches between the two layouts of a pair
    fn toggle(&self) -> Self {
        match self {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Grid,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Vertical,
        }
    }
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

// Image entities in display order: an image's position in the list is the cell it's shown in.
// Reordering or removing an image is just a Vec operation, nothing else to renumber.
#[derive(Resource, Default)]
struct ImageList(Vec<Entity>);

impl ImageList {
    fn slot(&self, entity: Entity) -> Option<usize> {
        self.0.iter().position(|&e| e == entity)
    }
}

#[derive(Resource)]
struct ReviewState {
    enabled: bool,
    directory: String,
    cell_patterns: Vec<review::CellPattern>,
    radixes: Vec<String>,
    current_index: usize,
    editable_patterns: Vec<String>,
    error: Option<String>,
    // When enabled, dropping an image switches to the set that image belongs to
    // instead of replacing the current batch. On by default.
    drag_switch: bool,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: String::new(),
            cell_patterns: Vec::new(),
            radixes: Vec::new(),
            current_index: 0,
            editable_patterns: Vec::new(),
            error: None,
            drag_switch: true,
        }
    }
}

impl ReviewState {
    // Replace the open images with the files of the set identified by `radix`
    fn load_set(&self, radix: &str) -> LoadImagesEvent {
        let directory = PathBuf::from(&self.directory);
        let mut paths = Vec::new();
        for file in review::resolve_files_for_radix(&directory, radix, &self.cell_patterns) {
            let Some(path) = file else { continue };
            paths.push(path);
        }
        LoadImagesEvent { paths, replace: true }
    }
}

// Snapshot of the current zoom/pan, captured when switching review sets so the next
// batch reuses it instead of auto-fitting. Indexed by display slot.
#[derive(Resource, Default)]
struct PreservedView {
    pending: bool,
    global_scale: f32,
    cells: Vec<(f32, Vec2)>,
}

// MARK: Components
// Links a label or cursor entity to the image it belongs to
#[derive(Component)]
struct ImageOf(Entity);

#[derive(Component)]
struct Scale(f32);

#[derive(Component)]
struct Position(Vec2);

// Rotation in quarter turns, clockwise (1 is 90°)
#[derive(Component)]
struct Rotation(i32);

#[derive(Component)]
struct ImagePath(String);

#[derive(Component)]
struct MyCursor;

#[derive(Component)]
struct MyImage;

// Full image size in pixels. The image entity has no Sprite of its own (the texture lives on tile children).
#[derive(Component)]
struct ImageSize(Vec2);

// Region of the image currently shown in its cell, in image pixel coordinates
#[derive(Component, Default)]
struct ViewRect(Option<Rect>);

// Child of a MyImage entity holding one texture tile of the image
#[derive(Component)]
struct MyTile;

// Area of the full image covered by a tile, in image pixel coordinates
#[derive(Component)]
struct TileRect(Rect);

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
struct SaveCropped;

#[derive(Message)]
struct ResetScales;

#[derive(Message)]
struct FitToScreen;

#[derive(Message)]
struct ChangeSamplerEvent;

struct ImageTile {
    handle: Handle<Image>,
    rect: Rect,
}

// Loaded images are appended to ImageList in the order their events are read.
// `replace` is set on the first image of a replacing batch: the current images are dropped before it's added.
#[derive(Message)]
struct NewImageLoadedEvent {
    tiles: Vec<ImageTile>,
    size: Vec2,
    path: String,
    replace: bool,
}

// Load a batch of images, either replacing the current ones or appended after them (add mode).
// A replacing batch where nothing decodes leaves the current images untouched.
#[derive(Message)]
struct LoadImagesEvent {
    paths: Vec<String>,
    replace: bool,
}

#[derive(Message)]
struct RemoveImageEvent(Entity); // the image entity to remove

// Reload every open image from disk, e.g. after the screen color space
// changed and the decoded pixels must be reconverted.
#[derive(Message)]
struct ReloadImagesEvent;

#[derive(Message)]
struct NavigateReviewEvent(i32); // +1 next, -1 previous

#[derive(Message)]
struct RefreshReviewEvent;

#[derive(Message)]
struct ActivateReviewEvent;

// Drag-to-switch: jump to the review set that the dropped file belongs to.
#[derive(Message)]
struct SwitchReviewToFileEvent(String);

// MARK: Setup
fn setup(
    mut commands: Commands,
    images_filename: ResMut<InitialImagesFilename>,
    config: Res<Config>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let mut camera = commands.spawn(Camera2d);
    if config.hdr.enabled {
        camera.insert(Hdr);
    }

    let bytes = include_bytes!("../assets/fonts/IBMPlexMono-Regular.otf");
    let font = Font::from_bytes(bytes.to_vec());
    let font_handle = fonts.add(font);
    commands.spawn(FontHandle(font_handle.clone()));

    commands.spawn((
        Text::new(HELP_STRING),
        TextFont {
            font: font_handle.into(),
            font_size: FontSize::Px(15.0),
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

    load_image_evw.write(LoadImagesEvent {
        paths: images_filename.0.clone(),
        replace: true,
    });
}

// MARK: Settings
fn save_settings(mut save_settings_evr: MessageReader<SaveSettingsEvent>, config: Res<Config>) {
    for _ev in save_settings_evr.read() {
        let Some(dst_path) = config_path() else {
            return;
        };
        let Ok(mut file) = File::create(&dst_path) else {
            println!("Failed to create config file: {}", dst_path.display());
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

// MARK: Utilities
fn redraw_while_pending<T: Message>(messages: Res<Messages<T>>, mut redraw_evw: MessageWriter<RequestRedraw>) {
    if !messages.is_empty() {
        redraw_evw.write(RequestRedraw);
    }
}

fn check_all_images_exist(images: &[String]) -> Result<Vec<String>> {
    let mut images_absolute = Vec::new();
    for image_filename in images {
        let input_path = Path::new(&image_filename);
        if !input_path.exists() {
            return Err(format!("Image not found: {}", image_filename).into());
        }
        if !input_path.is_file() {
            return Err(format!("Provided Path is not a file: {}", image_filename).into());
        }
        let resolved_path = canonicalize(input_path)?;
        let Some(image_absolute) = resolved_path.as_path().to_str() else {
            return Err(format!("Can't resolve given path: {}", image_filename).into());
        };
        images_absolute.push(String::from(image_absolute));
    }
    Ok(images_absolute)
}

// User config file, ~/.image_viewer
fn config_path() -> Option<PathBuf> {
    let Some(home_directory) = home_dir() else {
        println!("User directory not found");
        return None;
    };
    let config_path = home_directory.join(".image_viewer");
    println!("Config path: {}", config_path.display());
    Some(config_path)
}

fn get_short_name(path: &str) -> Option<&str> {
    Path::new(path).file_name()?.to_str()
}
