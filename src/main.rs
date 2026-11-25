// #![allow(unused_variables)]
#![windows_subsystem = "windows"]

mod review;

use std::env::home_dir;
use std::f32::consts::{PI, TAU};
use std::fs::canonicalize;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::window::{PresentMode, WindowResized, WindowResolution};
use bevy_egui::egui::CollapsingHeader;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use clap::Parser;
use image::{ColorType, DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, Pixel};
use moxcms::{CmsError, ColorProfile, DataColorSpace, Layout, TransformExecutor, TransformOptions};
use serde::{Deserialize, Serialize};

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
        .run();

    Ok(())
}

// MARK: Color Management
// Decoded images are converted from their color space (embedded ICC profile,
// or sRGB when untagged) to the configured screen color space, so all images
// are compared in the same space and displayed with the intended colors.

fn convert_to_screen_space(
    mut image: DynamicImage,
    icc: Option<&[u8]>,
    target: ScreenColorSpace,
    path: &str,
) -> DynamicImage {
    let src_profile = match icc {
        Some(icc) => match ColorProfile::new_from_slice(icc) {
            Ok(profile) => profile,
            Err(e) => {
                println!("{path}: ignoring unreadable ICC profile: {e}");
                return image;
            }
        },
        // Untagged images are assumed sRGB: already in place for an sRGB
        // screen, otherwise they need the conversion like everything else.
        None if target == ScreenColorSpace::Srgb => return image,
        None => ColorProfile::new_srgb(),
    };

    let is_gray = matches!(
        image.color(),
        ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16
    );
    // The decoded buffers are RGB or grayscale; a profile for another space
    // (e.g. CMYK, the decoder already converted those pixels) can't apply.
    let expected_space = if is_gray {
        DataColorSpace::Gray
    } else {
        DataColorSpace::Rgb
    };
    if src_profile.color_space != expected_space {
        println!(
            "{path}: ICC profile is for {:?} but pixels are {:?}, profile ignored",
            src_profile.color_space,
            image.color()
        );
        return image;
    }

    let dst_profile = target.profile();
    let result = match &mut image {
        DynamicImage::ImageLuma8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLumaA8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgb8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgba8(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLuma16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageLumaA16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgb16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        DynamicImage::ImageRgba16(buf) => icc_convert_image(&src_profile, &dst_profile, buf),
        // f32 formats are not loaded by this app
        _ => Ok(()),
    };
    if let Err(e) = result {
        println!("{path}: color space conversion failed, image kept as-is: {e}");
    }
    image
}

// Sample types moxcms can transform; picks the executor matching the bit depth.
trait IccSample: image::Primitive + Default {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<Self> + Send + Sync>, CmsError>;
}

impl IccSample for u8 {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<u8> + Send + Sync>, CmsError> {
        src.create_transform_8bit(layout, dst, layout, TransformOptions::default())
    }
}

impl IccSample for u16 {
    fn create_transform(
        src: &ColorProfile,
        dst: &ColorProfile,
        layout: Layout,
    ) -> std::result::Result<Arc<dyn TransformExecutor<u16> + Send + Sync>, CmsError> {
        src.create_transform_16bit(layout, dst, layout, TransformOptions::default())
    }
}

fn icc_convert_image<P>(
    src: &ColorProfile,
    dst: &ColorProfile,
    buf: &mut ImageBuffer<P, Vec<P::Subpixel>>,
) -> std::result::Result<(), CmsError>
where
    P: Pixel,
    P::Subpixel: IccSample,
{
    let layout = match P::CHANNEL_COUNT {
        1 => Layout::Gray,
        2 => Layout::GrayAlpha,
        3 => Layout::Rgb,
        _ => Layout::Rgba,
    };
    let row_len = buf.width() as usize * P::CHANNEL_COUNT as usize;
    let transform = P::Subpixel::create_transform(src, dst, layout)?;
    transform_rows_in_place(transform.as_ref(), buf, row_len)
}

// The executor needs distinct source and destination slices; going row by row
// through a scratch buffer avoids duplicating the whole image.
fn transform_rows_in_place<V: Copy + Default>(
    transform: &dyn TransformExecutor<V>,
    data: &mut [V],
    row_len: usize,
) -> std::result::Result<(), CmsError> {
    let mut scratch = vec![V::default(); row_len];
    for row in data.chunks_exact_mut(row_len) {
        scratch.copy_from_slice(row);
        transform.transform(&scratch, row)?;
    }
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

struct PanelRootUi<'a> {
    ui: egui::Ui,
    area: &'a mut UiPanelArea,
}

impl<'a> PanelRootUi<'a> {
    fn new(ctx: &egui::Context, id: &str, area: &'a mut UiPanelArea) -> Self {
        let pass_nr = ctx.cumulative_pass_nr();
        if area.pass_nr != pass_nr {
            area.pass_nr = pass_nr;
            area.rect = None;
        }
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new(id),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(area.rect.unwrap_or_else(|| ctx.viewport_rect())),
        );
        PanelRootUi { ui, area }
    }
}

impl Drop for PanelRootUi<'_> {
    fn drop(&mut self) {
        self.area.rect = Some(self.ui.available_rect_before_wrap());
    }
}

impl std::ops::Deref for PanelRootUi<'_> {
    type Target = egui::Ui;
    fn deref(&self) -> &egui::Ui {
        &self.ui
    }
}

impl std::ops::DerefMut for PanelRootUi<'_> {
    fn deref_mut(&mut self) -> &mut egui::Ui {
        &mut self.ui
    }
}

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

// Rotation in quarter turn (1 is 1 turn)
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
    mut ui_state: ResMut<UiState>,
    mut global_scale: ResMut<GlobalScale>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut save_cropped_evw: MessageWriter<SaveCropped>,
    mut reset_scales_evw: MessageWriter<ResetScales>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
    mut add_mode: ResMut<AddMode>,
    mut review_state: ResMut<ReviewState>,
    mut activate_evw: MessageWriter<ActivateReviewEvent>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if ui_state.visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        let mut root = PanelRootUi::new(ctx, "root_bottom_menu", &mut panel_area);
        egui::Panel::bottom("wrap_app_top_bar").show(&mut root, |ui| {
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
                            move_image_evw.write(MoveImageEvent);
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
                        move_image_evw.write(MoveImageEvent);
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
                    let review_toggled = ui
                        .toggle_value(&mut review_state.enabled, "Review")
                        .on_hover_text("Review mode: navigate through similar images")
                        .changed();
                    if review_toggled && review_state.enabled && review_state.radixes.is_empty() {
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
    mut change_title_style_evw: MessageWriter<ChangeTitleStyleEvent>,
    mut save_settings_evw: MessageWriter<SaveSettingsEvent>,
    mut change_sampler_evw: MessageWriter<ChangeSamplerEvent>,
    mut reload_images_evw: MessageWriter<ReloadImagesEvent>,
    mut next_state: ResMut<NextState<MyAppState>>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if ui_state.settings_panel_visible {
        let Ok(ctx) = contexts.ctx_mut() else { return };
        let mut root = PanelRootUi::new(ctx, "root_settings", &mut panel_area);
        egui::Panel::right("Settings").resizable(false).show(&mut root, |ui| {
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

                ui.horizontal(|ui| {
                    ui.label("Screen Color Space:")
                        .on_hover_text("Color space of the physical screen; images are converted to it");
                    egui::ComboBox::from_id_salt("screen_color_space")
                        .selected_text(config.color.screen_color_space.label())
                        .show_ui(ui, |ui| {
                            let mut changed = false;
                            for space in ScreenColorSpace::ALL {
                                changed |= ui
                                    .selectable_value(&mut config.color.screen_color_space, space, space.label())
                                    .changed();
                            }
                            if changed {
                                reload_images_evw.write(ReloadImagesEvent);
                            }
                        });
                });

                ui.checkbox(&mut cursor_state.0, "Enable Multi Cursor");

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

fn ui_edit_short_cut(mut contexts: EguiContexts, mut panel_area: ResMut<UiPanelArea>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut root = PanelRootUi::new(ctx, "root_edit_short_cut", &mut panel_area);
    egui::CentralPanel::default().show(&mut root, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
            ui.heading("Press short cut key");
        })
    });
}

fn ui_image_list_panel(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut image_list: ResMut<ImageList>,
    image_path_query: Query<&ImagePath, With<MyImage>>,
    mut remove_image_evw: MessageWriter<RemoveImageEvent>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut review_state: ResMut<ReviewState>,
    mut panel_area: ResMut<UiPanelArea>,
) {
    if !ui_state.image_list_visible {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let mut root = PanelRootUi::new(ctx, "root_image_list", &mut panel_area);
    egui::Panel::left("Image List")
        .resizable(true)
        .default_size(180.)
        .show(&mut root, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Images");
            });
            ui.separator();

            let mut to_remove: Option<Entity> = None;
            let mut from_slot: Option<usize> = None;
            let mut to_slot: Option<usize> = None;

            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for (slot, &image_entity) in image_list.0.iter().enumerate() {
                    let name = 'name: {
                        let Ok(path) = image_path_query.get(image_entity) else {
                            break 'name "?";
                        };
                        get_short_name(&path.0).unwrap_or("?")
                    };

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
                            to_remove = Some(image_entity);
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
                    move_item(&mut image_list.0, src, dst);
                    // Review patterns are per cell, they follow their image
                    move_item(&mut review_state.cell_patterns, src, dst);
                    move_item(&mut review_state.editable_patterns, src, dst);
                    move_image_evw.write(MoveImageEvent);
                }
            }

            // Handle removal
            if let Some(entity_to_remove) = to_remove {
                remove_image_evw.write(RemoveImageEvent(entity_to_remove));
            }
        });
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

fn on_remove_image(
    mut remove_evr: MessageReader<RemoveImageEvent>,
    mut commands: Commands,
    title_query: Query<(Entity, &ImageOf), With<MyText>>,
    mut image_list: ResMut<ImageList>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut fit_to_screen_evw: MessageWriter<FitToScreen>,
    mut review_state: ResMut<ReviewState>,
) {
    for ev in remove_evr.read() {
        let target = ev.0;
        let Some(slot) = image_list.slot(target) else {
            continue;
        };
        image_list.0.remove(slot);

        // The image despawns with its tiles; its label is a separate entity (cursors are handled by sync_cursors)
        commands.entity(target).despawn();
        for (entity, owner) in &title_query {
            if owner.0 == target {
                commands.entity(entity).despawn();
            }
        }

        // Review patterns are per cell, they follow their image even while review is off,
        // same as on reorder, so they still line up if review is turned back on
        if slot < review_state.cell_patterns.len() {
            review_state.cell_patterns.remove(slot);
        }
        if slot < review_state.editable_patterns.len() {
            review_state.editable_patterns.remove(slot);
        }

        fit_to_screen_evw.write(FitToScreen);
        move_image_evw.write(MoveImageEvent);
    }
}

// Decode an image from disk and convert it from its own color space to `target`
fn decode_image(path: &str, target: ScreenColorSpace) -> Option<DynamicImage> {
    let Some(f) = File::open(path).ok() else {
        println!("Failed to open file: {}", path);
        return None;
    };
    let Some(format) = ImageFormat::from_path(path).ok() else {
        println!("Failed to deduce image format from path: {}", path);
        return None;
    };

    let mut reader = image::ImageReader::with_format(BufReader::new(f), format);

    // Remove the memory limit on image size we can read
    // This is required to process large images that would otherwise be rejected by the image crate
    reader.no_limits();

    let Some(mut decoder) = reader.into_decoder().ok() else {
        println!("Failed to decode image: {}", path);
        return None;
    };
    let icc_profile = decoder.icc_profile().ok().flatten();
    let Some(image) = DynamicImage::from_decoder(decoder).ok() else {
        println!("Failed to decode image: {}", path);
        return None;
    };
    let icc = icc_profile.as_deref().filter(|icc| !icc.is_empty());
    Some(convert_to_screen_space(image, icc, target, path))
}

fn on_load_image(
    mut load_evr: MessageReader<LoadImagesEvent>,
    mut loaded_evw: MessageWriter<NewImageLoadedEvent>,
    mut images: ResMut<Assets<Image>>,
    config: Res<Config>,
    render_device: Option<Res<RenderDevice>>,
) {
    let max_texture_size = match render_device {
        Some(device) => device.limits().max_texture_dimension_2d,
        None => FALLBACK_MAX_TEXTURE_SIZE,
    };
    let sampler = config.misc.sampler_mode.sampler();
    for ev in load_evr.read() {
        // Only the first image that actually loads replaces the current batch
        let mut replace = ev.replace;
        for path in &ev.paths {
            let Some(image) = decode_image(path, config.color.screen_color_space) else {
                continue;
            };

            let size = Vec2::new(image.width() as f32, image.height() as f32);
            let tile_rects = tile_grid(image.width(), image.height(), max_texture_size);
            // A single tile takes the decoded buffer as is; larger images are copied tile by tile
            let tile_images: Vec<(URect, DynamicImage)> = if tile_rects.len() == 1 {
                vec![(tile_rects[0], image)]
            } else {
                tile_rects
                    .into_iter()
                    .map(|r| (r, image.crop_imm(r.min.x, r.min.y, r.width(), r.height())))
                    .collect()
            };

            let mut tiles = Vec::with_capacity(tile_images.len());
            for (rect, tile_image) in tile_images {
                let Some(mut texture) = to_texture(tile_image) else {
                    break;
                };
                texture.sampler = sampler.clone();
                tiles.push(ImageTile {
                    handle: images.add(texture),
                    rect: rect.as_rect(),
                });
            }
            if tiles.is_empty() {
                continue;
            }
            loaded_evw.write(NewImageLoadedEvent {
                tiles,
                size,
                path: path.clone(),
                replace,
            });
            replace = false;
        }
    }
}

// Split a width x height image into the fewest equally sized tiles whose sides fit in max_size
fn tile_grid(width: u32, height: u32, max_size: u32) -> Vec<URect> {
    let cols = width.div_ceil(max_size).max(1);
    let rows = height.div_ceil(max_size).max(1);
    let tile_size = UVec2::new(width.div_ceil(cols), height.div_ceil(rows));
    let image_size = UVec2::new(width, height);
    let mut rects = Vec::with_capacity((cols * rows) as usize);
    for row in 0..rows {
        for col in 0..cols {
            let min = UVec2::new(col, row) * tile_size;
            let max = (min + tile_size).min(image_size);
            rects.push(URect { min, max });
        }
    }
    rects
}

fn to_texture(image: DynamicImage) -> Option<Image> {
    let usages = RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD;
    match image.color() {
        ColorType::Rgb8 | ColorType::Rgba8 | ColorType::L8 | ColorType::La8 => {
            Some(Image::from_dynamic(image, true, usages))
        }
        ColorType::Rgb16 | ColorType::Rgba16 => Some(Image::from_dynamic(image, true, usages)),
        ColorType::L16 => {
            let image_rgb16 = DynamicImage::ImageRgb16(image.into_rgb16());
            Some(Image::from_dynamic(image_rgb16, true, usages))
        }
        _ => {
            println!("Unsupported image type : image.color(): {:?}", image.color());
            None
        }
    }
}

fn on_image_loaded(
    config: Res<Config>,
    mut load_image_evr: MessageReader<NewImageLoadedEvent>,
    mut commands: Commands,
    title_query: Query<Entity, With<MyText>>,
    mut help_query: Query<&mut Visibility, With<MyHelp>>,
    font_query: Query<&FontHandle>,
    mut image_list: ResMut<ImageList>,
) {
    let events: Vec<&NewImageLoadedEvent> = load_image_evr.read().collect();
    if events.is_empty() {
        return;
    }
    let font = font_query.single().unwrap();

    // Everything before the last replacing image would be dropped anyway, so clear once and
    // skip those (labels spawned earlier in this frame aren't visible to title_query yet)
    let mut start = 0;
    if let Some(last_replace) = events.iter().rposition(|ev| ev.replace) {
        start = last_replace;
        for &entity in &image_list.0 {
            commands.entity(entity).despawn();
        }
        for entity in &title_query {
            commands.entity(entity).despawn();
        }
        image_list.0.clear();
    }

    for ev in &events[start..] {
        // Start hidden; on_move_image will make it visible after positioning.
        // The image entity carries the view state; each tile child carries one texture.
        let image_entity = commands
            .spawn((
                Transform::default(),
                Visibility::Hidden,
                Scale(1.),
                Position(Vec2::ZERO),
                Rotation(0),
                ImagePath(ev.path.clone()),
                ImageSize(ev.size),
                ViewRect::default(),
                MyImage,
            ))
            .with_children(|parent| {
                for tile in &ev.tiles {
                    parent.spawn((
                        Sprite {
                            image: tile.handle.clone(),
                            ..default()
                        },
                        TileRect(tile.rect),
                        MyTile,
                    ));
                }
            })
            .id();

        let short_path = get_short_name(&ev.path).unwrap_or("");
        commands.spawn((
            Text::new(short_path),
            TextFont {
                font: font.0.clone().into(),
                font_size: FontSize::Px(config.text.font_size),
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
            ImageOf(image_entity),
            MyText,
        ));

        image_list.0.push(image_entity);
    }

    let mut help_visibility = help_query.single_mut().unwrap();
    *help_visibility = Visibility::Hidden;
}

fn on_move_image(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut sprite_position: Query<
        (
            &Position,
            &Scale,
            &Rotation,
            &ImageSize,
            &mut ViewRect,
            &mut Transform,
            &mut Visibility,
            &Children,
        ),
        With<MyImage>,
    >,
    mut tiles: Query<(&TileRect, &mut Sprite, &mut Transform, &mut Visibility), (With<MyTile>, Without<MyImage>)>,
    global_scale: Res<GlobalScale>,
    global_rotation: Res<GlobalRotation>,
    layout_state: Res<GridLayoutState>,
    mouse_state: Res<MouseState>,
    image_list: Res<ImageList>,
    mut title_query: Query<&mut Node, With<MyText>>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let window = windows.single().unwrap();
    if window.width() == 0. || window.height() == 0. {
        return;
    }
    let num_images = image_list.0.len();
    for (slot, &entity) in image_list.0.iter().enumerate() {
        // Images spawned this frame are listed before their components exist
        let Ok((position, scale, rotation, image_size, mut view_rect, mut transform, mut visibility, children)) =
            sprite_position.get_mut(entity)
        else {
            continue;
        };
        let image_size = image_size.0;

        let (cell_offset, cell_size) =
            get_cell_rect(slot, num_images, &layout_state.layout, window, config.misc.grid_width);
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

        let view = cell.intersect(image_crop);
        view_rect.0 = Some(view);

        // Each tile shows its share of the view, offset from the view center in image pixels.
        // The parent transform applies scale and rotation, so child translations stay in image space.
        for child in children.iter() {
            let Ok((tile, mut sprite, mut tile_transform, mut tile_visibility)) = tiles.get_mut(child) else {
                continue;
            };
            let tile_view = view.intersect(tile.0);
            if tile_view.is_empty() {
                *tile_visibility = Visibility::Hidden;
                continue;
            }
            *tile_visibility = Visibility::Inherited;
            sprite.rect = Some(Rect {
                min: tile_view.min - tile.0.min,
                max: tile_view.max - tile.0.min,
            });
            tile_transform.translation = ((tile_view.center() - view.center()) * Vec2::new(1., -1.)).extend(0.);
        }

        // Make visible after positioning (sprites start hidden to avoid flash at native resolution)
        visibility.set_if_neq(cell_visibility(slot, &layout_state));
    }

    let (_, cell_size) = get_cell_rect(0, num_images, &layout_state.layout, window, config.misc.grid_width);
    for mut node in &mut title_query {
        node.width = Val::Px(cell_size.x);
    }
}

// Only the selected image is shown in Stack layout
fn cell_visibility(slot: usize, layout_state: &GridLayoutState) -> Visibility {
    if layout_state.layout == GridLayout::Stack && slot != layout_state.index {
        Visibility::Hidden
    } else {
        Visibility::Visible
    }
}

fn on_move_image_title(
    mut move_image_evr: MessageReader<MoveImageEvent>,
    windows: Query<&Window>,
    mut text_query: Query<(&ImageOf, &mut Node, &mut Visibility), With<MyText>>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    config: Res<Config>,
) {
    if move_image_evr.is_empty() {
        return;
    }
    move_image_evr.clear();

    let num_images = image_list.0.len();
    let window = windows.single().unwrap();

    for (owner, mut node, mut visibility) in &mut text_query {
        let Some(slot) = image_list.slot(owner.0) else {
            continue;
        };
        let (cell_offset, _) = get_cell_rect(slot, num_images, &layout_state.layout, window, config.misc.grid_width);
        node.top = Val::Px(cell_offset.y + 2.);
        node.left = Val::Px(cell_offset.x + 5.);
        visibility.set_if_neq(cell_visibility(slot, &layout_state));
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
        text_font.font_size = FontSize::Px(config.text.font_size);
    }
}

fn on_move_cursor(
    windows: Query<&Window>,
    mut cursor_query: Query<(&ImageOf, &mut Transform, &mut Visibility), With<MyCursor>>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    config: Res<Config>,
) {
    let num_images = image_list.0.len();
    let window = windows.single().unwrap();
    if window.width() == 0. || window.height() == 0. {
        return;
    }

    let cursor_position = window.cursor_position();
    for (owner, mut transform, mut visibility) in &mut cursor_query {
        let Some(slot) = image_list.slot(owner.0) else {
            continue;
        };
        visibility.set_if_neq(cell_visibility(slot, &layout_state));

        let Some(cursor_position) = cursor_position else {
            continue;
        };
        let (cell_offset, cell_size) =
            get_cell_rect(slot, num_images, &layout_state.layout, window, config.misc.grid_width);
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

fn key_change_layout(
    config: Res<Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut layout_state: ResMut<GridLayoutState>,
) {
    if keys.just_pressed(config.shortcut.switch_layout) {
        layout_state.layout = match layout_state.layout {
            GridLayout::Grid => GridLayout::Stack,
            GridLayout::Stack => GridLayout::Vertical,
            GridLayout::Vertical => GridLayout::Horizontal,
            GridLayout::Horizontal => GridLayout::Grid,
        };
        move_image_evw.write(MoveImageEvent);
    }
}

fn change_layout_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
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
        move_image_evw.write(MoveImageEvent);
    }
}

fn change_top_image(
    keys: Res<ButtonInput<KeyCode>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
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
    image_list: Res<ImageList>,
    mut sprite_query: Query<&mut Rotation, With<MyImage>>,
) {
    if !keys.pressed(config.shortcut.local_rotate_modifier) {
        return;
    }
    if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
        return;
    }
    let window = windows.single().unwrap();
    let Some(entity) = image_under_cursor(window, &image_list, &layout_state, config.misc.grid_width) else {
        return;
    };
    let Ok(mut rotate) = sprite_query.get_mut(entity) else {
        return;
    };
    rotate.0 += if buttons.just_pressed(MouseButton::Left) { 1 } else { -1 };
    move_image_evw.write(MoveImageEvent);
}

fn change_zoom_individually(
    config: Res<Config>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    layout_state: Res<GridLayoutState>,
    image_list: Res<ImageList>,
    mut sprite_query: Query<(&mut Scale, &mut Position), With<MyImage>>,
) {
    if !keys.pressed(config.shortcut.local_zoom_modifier) {
        return;
    }
    if !(buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right)) {
        return;
    }
    let window = windows.single().unwrap();
    let Some(entity) = image_under_cursor(window, &image_list, &layout_state, config.misc.grid_width) else {
        return;
    };
    let Ok((mut scale, position)) = sprite_query.get_mut(entity) else {
        return;
    };
    scale.0 *= if buttons.just_pressed(MouseButton::Left) {
        2.
    } else {
        0.5
    };
    let position_normalized = position.0 * scale.0;

    // Reset position for other images to match the one we zoom
    for (scale, mut position) in &mut sprite_query {
        position.0 = position_normalized / scale.0;
    }
    move_image_evw.write(MoveImageEvent);
}

// Image whose cell is under the mouse cursor
fn image_under_cursor(
    window: &Window,
    image_list: &ImageList,
    layout_state: &GridLayoutState,
    grid_width: i32,
) -> Option<Entity> {
    let cursor_position = window.cursor_position()?;
    // Stacked cells all cover the window, only the selected one is visible
    if layout_state.layout == GridLayout::Stack {
        return image_list.0.get(layout_state.index).copied();
    }
    let num_images = image_list.0.len();
    for (slot, &entity) in image_list.0.iter().enumerate() {
        let (cell_offset, cell_size) = get_cell_rect(slot, num_images, &layout_state.layout, window, grid_width);
        if Rect::from_corners(cell_offset, cell_offset + cell_size).contains(cursor_position) {
            return Some(entity);
        }
    }
    None
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
    mut cursor_state: ResMut<MultiCursorEnabled>,
) {
    if keys.just_pressed(config.shortcut.switch_cursor) {
        cursor_state.0 = !cursor_state.0;
    }
}

// MultiCursorEnabled is the source of truth: this is the only system spawning or despawning cursors,
// so they follow the image list whatever happens to it (new batch, add mode, removal)
fn sync_cursors(
    mut commands: Commands,
    cursor_query: Query<(Entity, &ImageOf), With<MyCursor>>,
    cursor_state: Res<MultiCursorEnabled>,
    image_list: Res<ImageList>,
) {
    let mut has_cursor = Vec::new();
    for (entity, owner) in &cursor_query {
        if cursor_state.0 && image_list.slot(owner.0).is_some() {
            has_cursor.push(owner.0);
        } else {
            commands.entity(entity).despawn();
        }
    }
    if !cursor_state.0 {
        return;
    }

    for &image_entity in &image_list.0 {
        if has_cursor.contains(&image_entity) {
            continue;
        }
        commands
            .spawn((
                Transform::from_translation(Vec3::new(0., 0., 1.)),
                Visibility::default(),
                ImageOf(image_entity),
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

fn on_image_spawned(mut fit_to_screen_evw: MessageWriter<FitToScreen>, sprite_query: Query<(), Added<MyImage>>) {
    if sprite_query.is_empty() {
        return;
    }

    fit_to_screen_evw.write(FitToScreen);
}

fn fit_to_screen(
    mut fit_to_screen_evr: MessageReader<FitToScreen>,
    windows: Query<&Window>,
    mut global_scale: ResMut<GlobalScale>,
    mut sprite_query: Query<(&ImageSize, &mut Scale, &mut Position), With<MyImage>>,
    image_list: Res<ImageList>,
    layout_state: Res<GridLayoutState>,
    config: Res<Config>,
    mut move_image_evw: MessageWriter<MoveImageEvent>,
    mut preserved: ResMut<PreservedView>,
) {
    for _ev in fit_to_screen_evr.read() {
        let window = windows.single().unwrap();
        if window.width() == 0. || window.height() == 0. {
            continue;
        }

        // When switching review sets we reuse the previous zoom/pan instead of refitting.
        if preserved.pending {
            global_scale.0 = preserved.global_scale;
            for (slot, &entity) in image_list.0.iter().enumerate() {
                let Ok((_size, mut scale, mut position)) = sprite_query.get_mut(entity) else {
                    continue;
                };
                if let Some(&(cell_scale, cell_position)) = preserved.cells.get(slot) {
                    scale.0 = cell_scale;
                    position.0 = cell_position;
                }
            }
            preserved.pending = false;
            move_image_evw.write(MoveImageEvent);
            continue;
        }

        let num_images = image_list.0.len();

        let mut first = true;
        for (slot, &entity) in image_list.0.iter().enumerate() {
            let Ok((image_size, mut scale, mut position)) = sprite_query.get_mut(entity) else {
                continue;
            };
            let image_size = image_size.0;
            let (_, cell_size) = get_cell_rect(slot, num_images, &layout_state.layout, window, config.misc.grid_width);
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
    image_query: Query<(&ImagePath, &ViewRect), With<MyImage>>,
) {
    for _ev in save_cropped_evr.read() {
        for (path, view_rect) in &image_query {
            let Some(rect) = view_rect.0 else {
                println!("Failed to get ROI of the texture: {}", path.0);
                continue;
            };
            let Some(output_path) = insert_suffix(Path::new(&path.0), "_crop") else {
                println!("Failed to create output filename: {}", path.0);
                continue;
            };

            // The crop is saved without a profile and may be opened anywhere,
            // so it is always converted to sRGB, not to the screen color space.
            let Some(image) = decode_image(&path.0, ScreenColorSpace::Srgb) else {
                continue;
            };
            let size = rect.size();
            let crop = image.crop_imm(rect.min.x as u32, rect.min.y as u32, size.x as u32, size.y as u32);

            // RGB8 is the one layout every encoder takes (JPEG has no alpha or 16 bits).
            // The format follows the extension, the same as the input.
            if let Err(e) = crop.to_rgb8().save(&output_path) {
                println!("Failed to write data to file {}: {}", output_path.display(), e);
            }
        }
    }
}

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

fn change_sampler(
    mut change_sampler_evr: MessageReader<ChangeSamplerEvent>,
    config: Res<Config>,
    sprite_query: Query<&Sprite, With<MyTile>>,
    mut images: ResMut<Assets<Image>>,
) {
    if change_sampler_evr.is_empty() {
        return;
    }
    change_sampler_evr.clear();

    let new_sampler = config.misc.sampler_mode.sampler();

    for sprite in &sprite_query {
        let Some(mut image) = images.get_mut(&sprite.image) else {
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
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    add_mode: Res<AddMode>,
    review_state: Res<ReviewState>,
    mut switch_review_evw: MessageWriter<SwitchReviewToFileEvent>,
) {
    let mut images_filename = Vec::new();
    for ev in dnd_evr.read() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = ev {
            // On Windows, explorer hands us 8.3 short names (FOO~1.PNG) when the full path
            // exceeds MAX_PATH. Canonicalizing resolves them back to the real long names.
            let Ok(resolved_path) = canonicalize(path_buf) else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            let Some(image_absolute) = resolved_path.as_path().to_str() else {
                println!("Can't resolve given path: {:?}", path_buf);
                continue;
            };
            images_filename.push(String::from(image_absolute));
        }
    }
    if images_filename.is_empty() {
        return;
    }

    // In review drag-to-switch mode, a dropped image jumps to the set it belongs to
    // rather than replacing the current batch. Only the first dropped file is used.
    if review_state.enabled && review_state.drag_switch {
        if let Some(first) = images_filename.into_iter().next() {
            switch_review_evw.write(SwitchReviewToFileEvent(first));
        }
        return;
    }

    // In add mode the new images are appended after the current ones
    load_image_evw.write(LoadImagesEvent {
        paths: images_filename,
        replace: !add_mode.0,
    });
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
        for url in urls.iter() {
            let Some(ns_path) = url.path() else {
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
        let Some(cls) = AnyClass::get(c"WinitApplicationDelegate") else {
            println!("macOS dock drop: WinitApplicationDelegate class not found, skipping");
            return;
        };

        let sel = sel!(application:openURLs:);
        // Type encoding: void(id, SEL, id, id) = "v@:@@"
        let types = c"v@:@@";
        let imp: Imp = unsafe { std::mem::transmute(handle_open_urls as *const ()) };
        let cls_ptr = cls as *const AnyClass as *mut AnyClass;

        // SAFETY: cls_ptr points to a valid, registered ObjC class. The selector, type encoding,
        // and function signature all match the application:openURLs: delegate method.
        let ok = unsafe { ffi::class_addMethod(cls_ptr, sel, imp, types.as_ptr()) };
        if ok.as_bool() {
            println!("macOS dock drop: injected application:openURLs: handler");
        } else {
            println!("macOS dock drop: failed to inject handler (method may already exist)");
        }
    }
}

// Reload all open images from disk, preserving the current zoom/pan.
// Needed when the screen color space changes: conversion happens at decode
// time, so the pixels already uploaded are in the previous space.
fn on_reload_images(
    mut reload_evr: MessageReader<ReloadImagesEvent>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    view_query: Query<(&Scale, &Position), With<MyImage>>,
    path_query: Query<&ImagePath, With<MyImage>>,
) {
    if reload_evr.is_empty() {
        return;
    }
    reload_evr.clear();

    if image_list.0.is_empty() {
        return;
    }

    capture_view(&mut preserved, global_scale.0, &image_list, &view_query);
    let mut paths = Vec::new();
    for &entity in &image_list.0 {
        let Ok(path) = path_query.get(entity) else {
            continue;
        };
        paths.push(path.0.clone());
    }
    load_image_evw.write(LoadImagesEvent { paths, replace: true });
}

fn poll_dock_drop_queue(mut load_image_evw: MessageWriter<LoadImagesEvent>, add_mode: Res<AddMode>) {
    let Ok(mut queue) = DOCK_DROP_QUEUE.lock() else {
        return;
    };
    if queue.is_empty() {
        return;
    }
    load_image_evw.write(LoadImagesEvent {
        paths: queue.drain(..).collect(),
        replace: !add_mode.0,
    });
}

// MARK: Review Mode

// Snapshot the current zoom/pan of the open images so the next review set reuses it
// instead of auto-fitting. Cleared/overwritten on every set switch.
fn capture_view(
    preserved: &mut PreservedView,
    global_scale: f32,
    image_list: &ImageList,
    sprite_query: &Query<(&Scale, &Position), With<MyImage>>,
) {
    preserved.global_scale = global_scale;
    preserved.cells.clear();
    for &entity in &image_list.0 {
        let Ok((scale, position)) = sprite_query.get(entity) else {
            continue;
        };
        preserved.cells.push((scale.0, position.0));
    }
    preserved.pending = !preserved.cells.is_empty();
}

fn on_navigate_review(
    mut navigate_evr: MessageReader<NavigateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    sprite_query: Query<(&Scale, &Position), With<MyImage>>,
) {
    for ev in navigate_evr.read() {
        if review_state.radixes.is_empty() {
            continue;
        }
        capture_view(&mut preserved, global_scale.0, &image_list, &sprite_query);
        let count = review_state.radixes.len() as i32;
        let new_index = (review_state.current_index as i32 + ev.0).rem_euclid(count) as usize;
        review_state.current_index = new_index;
        load_image_evw.write(review_state.load_set(&review_state.radixes[new_index]));
    }
}

fn on_switch_review_to_file(
    mut switch_evr: MessageReader<SwitchReviewToFileEvent>,
    mut review_state: ResMut<ReviewState>,
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
    mut preserved: ResMut<PreservedView>,
    global_scale: Res<GlobalScale>,
    image_list: Res<ImageList>,
    sprite_query: Query<(&Scale, &Position), With<MyImage>>,
) {
    for ev in switch_evr.read() {
        let Some(filename) = Path::new(&ev.0).file_name().and_then(|name| name.to_str()) else {
            review_state.error = Some("Cannot read the dropped file name".to_string());
            continue;
        };

        let Some(radix) = review::match_radix(filename, &review_state.cell_patterns) else {
            review_state.error = Some("Dropped image doesn't match the review pattern".to_string());
            continue;
        };

        capture_view(&mut preserved, global_scale.0, &image_list, &sprite_query);

        // Locate the set in the scanned list so the counter stays in sync. If it isn't there
        // (e.g. dropped from outside the reviewed directory) we still load it, leaving the index.
        if let Some(index) = review_state.radixes.iter().position(|r| r == &radix) {
            review_state.current_index = index;
        }

        review_state.error = None;
        load_image_evw.write(review_state.load_set(&radix));
    }
}

fn on_activate_review(
    mut activate_evr: MessageReader<ActivateReviewEvent>,
    mut review_state: ResMut<ReviewState>,
    image_list: Res<ImageList>,
    image_query: Query<&ImagePath, With<MyImage>>,
) {
    for _ev in activate_evr.read() {
        review_state.error = None;
        // Paths in display order so pattern order matches the cells
        let mut paths = Vec::new();
        for &entity in &image_list.0 {
            let Ok(path) = image_query.get(entity) else {
                continue;
            };
            paths.push(path.0.clone());
        }
        if paths.len() < 2 {
            review_state.error = Some("Review mode needs at least 2 images".to_string());
            continue;
        }

        // Extract directory from the first image path
        let Some(dir) = Path::new(&paths[0]).parent() else {
            review_state.error = Some("Cannot determine directory from image path".to_string());
            continue;
        };

        let path_refs: Vec<&Path> = paths.iter().map(Path::new).collect();

        let Some(result) = review::extract_patterns_from_paths(&path_refs) else {
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
    mut load_image_evw: MessageWriter<LoadImagesEvent>,
) {
    for _ev in refresh_evr.read() {
        // Rebuild cell_patterns from editable_patterns
        let mut new_patterns = Vec::new();
        for (i, regex_str) in review_state.editable_patterns.iter().enumerate() {
            let old = review_state.cell_patterns.get(i);
            let directory = old.map(|cp| cp.directory.clone()).unwrap_or_default();
            let tail = old.map(|cp| cp.tail.clone()).unwrap_or_default();
            new_patterns.push(review::CellPattern {
                directory,
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
            load_image_evw.write(review_state.load_set(radix));
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
    mut panel_area: ResMut<UiPanelArea>,
) {
    if !ui_state.visible || !review_state.enabled {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut root = PanelRootUi::new(ctx, "root_review", &mut panel_area);
    egui::Panel::bottom("review_bar").show(&mut root, |ui| {
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

                ui.toggle_value(&mut review_state.drag_switch, "\u{1F4E5}")
                    .on_hover_text("Drag-to-switch: drop an image to jump to its set");

                let total = review_state.radixes.len();
                let current = review_state.current_index;
                let radix_name = review_state.radixes.get(current).cloned().unwrap_or_default();
                ui.label(format!("{}/{}: {}", current + 1, total, radix_name));

                ui.separator();

                for pattern in review_state.editable_patterns.iter_mut() {
                    ui.add(egui::TextEdit::singleline(pattern).desired_width(200.0));
                    ui.separator();
                }

                if ui
                    .button("\u{1F4E4}")
                    .on_hover_text("Reload directory with current regexes")
                    .clicked()
                {
                    refresh_evw.write(RefreshReviewEvent);
                }
            }

            if ui
                .button("\u{21BB}")
                .on_hover_text("Recompute patterns from open images")
                .clicked()
            {
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
