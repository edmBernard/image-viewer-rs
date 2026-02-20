# Image Viewer

A cross-platform image viewer for comparing images side-by-side, in grids, stacked, or in horizontal/vertical layouts. Built with Rust and Bevy.

- Github: [https://github.com/edmBernard/image-viewer-rs](https://github.com/edmBernard/image-viewer-rs)

## Usage

### Open Images

- **Command line**: `image-viewer img1.png img2.png ...`
- **Drag and drop**: Drag images from the file explorer into the window. Multiple images can be dropped at once.
- **macOS**: Drop images onto the app icon in the Dock or use "Open With".

By default, dropping new images replaces the current set. Enable **Add Mode** (`Q` key or the "Add" toggle in the bottom bar) to append images instead.

### Layouts

Four layout modes are available:

| Layout | Description |
|--------|-------------|
| **Grid** | Images arranged in a grid (auto-sized or configurable width) |
| **Stack** | One image at a time, switch with `Shift+1..9` or the bottom bar |
| **Horizontal** | Images side by side horizontally |
| **Vertical** | Images stacked vertically |

- Press `L` to cycle through layouts.
- **Double-click** to toggle between Grid/Stack or Horizontal/Vertical.

### Zoom

- **Scroll wheel**: Zoom in/out (when scroll behavior is set to "Zoom" in settings).
- **Ctrl/Cmd + 1..5**: Set zoom to 1x, 2x, 4x, 8x, 16x.
- **Ctrl/Cmd + Shift + 1..5**: Set zoom to 1/2, 1/4, 1/8, 1/16, 1/32.
- **Per-image zoom**: Hold `Z` (default) + left/right click on an image to zoom in/out that image only.
- **1:1** button in the bottom bar: Reset all zoom to 1:1.
- **Fit** button in the bottom bar: Fit images to their cells.

### Rotation

- Press `R` to rotate all images 90 degrees clockwise.
- Hold `E` (default) + left/right click on an image to rotate that image CW/CCW individually.

### Pan

- **Click and drag** to pan all images simultaneously.
- **Scroll wheel**: Pan images (when scroll behavior is set to "Move" in settings).

### Multi Cursor

Press `C` to toggle synchronized cursors across all images. Useful for comparing the same region in different images.

### Image List Panel

Click the hamburger icon (`☰`) in the bottom bar to open the image list panel on the left side. From there you can:

- **Reorder images** by drag and drop.
- **Remove images** by clicking the `✖` button next to each image.

### Save Cropped Images

Press `P` or click the save icon (`⛶`) in the bottom bar to save the currently visible crop of each image to disk. Files are saved next to the originals with a `_crop` suffix.

### Review Mode

Review mode lets you navigate through sets of related images in a directory. It automatically detects naming patterns from the currently loaded images and finds all matching sets.

1. Load 2 or more images that share a common naming pattern (e.g., `shot_001_diffuse.jpg` and `shot_001_specular.jpg`).
2. Click the **Review** toggle in the bottom bar.
3. The app detects the common radix and scans the directory for all matching sets.
4. Use the `◀` / `▶` buttons (or the review bar) to navigate between sets.
5. Regex patterns are shown and editable. Click `↻` to reload the directory after editing patterns.
6. Click `♲` to recompute patterns from the currently open images.

### Settings

Click the gear icon (`⚙`) in the bottom bar to open the settings panel:

- **Scroll behavior**: None, Move, or Zoom
- **Interpolation**: Nearest or Bilinear texture sampling
- **Multi cursor**: Toggle synchronized cursors
- **Grid width**: Set the number of columns in grid layout (0 = auto)
- **Font size and color**: Customize the image filename display
- **Keyboard shortcuts**: Click a shortcut button, then press a key to rebind it
- **Save Settings**: Persist settings to `~/.image_viewer`

### Toggle Interface

Press `H` to show/hide the entire UI (bottom bar, panels).

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `L` | Cycle layout (Grid -> Stack -> Vertical -> Horizontal) |
| Double-click | Toggle Grid/Stack or Horizontal/Vertical |
| `Shift + 1..9` | Switch to image N (in Stack layout, selects visible image) |
| `Ctrl/Cmd + 1..5` | Zoom 1x, 2x, 4x, 8x, 16x |
| `Ctrl/Cmd + Shift + 1..5` | Zoom 1/2, 1/4, 1/8, 1/16, 1/32 |
| `R` | Rotate all images CW |
| `E` + Left/Right click | Rotate hovered image CW/CCW |
| `Z` + Left/Right click | Zoom in/out hovered image only |
| `C` | Toggle multi cursor |
| `Q` | Toggle Add Mode |
| `P` | Save cropped images to disk |
| `H` | Toggle interface visibility |

All keyboard shortcuts can be remapped in the settings panel.

## Image Format Support

Image format support comes from the [image-rs](https://crates.io/crates/image) crate. Supported formats include JPEG, PNG, BMP, TIFF, EXR, and more. Images must be compatible with [wgpu](https://crates.io/crates/wgpu) textures. HDR rendering can be enabled in the config.

## Configuration

Create a config file named `.image_viewer` in your home directory. The default config is shown in [assets/default/config.toml](assets/default/config.toml).

Settings can also be changed at runtime through the settings panel and saved with the "Save Settings" button.

## Build

### Install Rust

Check official installation instructions: [get started](https://www.rust-lang.org/tools/install)

### Get Source

```
git clone git@github.com:edmBernard/image-viewer-rs
```

### Build

```bash
cargo build --release
```

The executable is named `image-viewer`.

### Bundle for macOS

Install [cargo-bundle](https://github.com/burtonageo/cargo-bundle) and run:

```bash
cargo bundle --release
```

This generates `image-viewer.app`. Bundling on macOS removes the background terminal and adds the app icon.

## Disclaimer

It's a toy project mainly used to learn Bevy. If you spot errors, improvement comments are welcome.
