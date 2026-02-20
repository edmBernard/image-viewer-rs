# Features

## Layouts

Four layout modes to compare images:

- **Grid**: Arranges images in a grid. The number of columns is auto-computed from the image count, or set manually via the "Grid Width" setting. Use grid layout when comparing many images at once.
- **Stack**: Shows one image at a time, filling the entire window. Switch between images using `Shift + 1..9` or the numbered buttons in the bottom bar. Useful for pixel-perfect A/B comparison in the same screen area.
- **Horizontal**: Places images side by side from left to right.
- **Vertical**: Places images top to bottom.

Press `L` to cycle through layouts. Double-click to quickly toggle between Grid/Stack or Horizontal/Vertical.

## Zoom

- **Global zoom**: All images share the same base zoom level. Use `Ctrl/Cmd + 1..5` for preset zoom levels (1x to 16x), or `Ctrl/Cmd + Shift + 1..5` for fractional zooms (1/2 to 1/32). The zoom drag value in the bottom bar allows fine-grained adjustment.
- **Per-image zoom**: Hold `Z` and left-click on an image to zoom it in, or right-click to zoom it out. Other images adjust their pan to stay synchronized.
- **Scroll zoom**: When scroll behavior is set to "Zoom" in settings, the mouse wheel zooms in and out.
- **Fit to screen**: Click the "Fit" button to scale all images to fit within their layout cells.
- **Reset**: Click "1:1" to reset all zoom to native resolution.

## Pan

Click and drag anywhere to pan all images simultaneously. The pan is synchronized: all images move together, which is useful for comparing the same region across different images. When scroll behavior is set to "Move", the mouse wheel pans vertically and horizontally.

## Rotation

- **Global rotation**: Press `R` to rotate all images 90 degrees clockwise.
- **Per-image rotation**: Hold `E` and left-click on an image to rotate it CW, or right-click for CCW. This lets you fix orientation on a per-image basis without affecting others.

## Multi Cursor

Press `C` or toggle the checkbox in settings to enable synchronized cursors. A red crosshair appears on each image, mirroring your mouse position relative to each image's cell. This makes it easy to inspect the exact same pixel location across multiple images.

## Add Mode

By default, dropping new images into the window replaces the current set. Toggle **Add Mode** with `Q` or the "Add" button in the bottom bar. When enabled, dropped images are appended to the existing set instead of replacing it.

## Image List Panel

Click the `☰` icon in the bottom bar to open the image list on the left side. This panel shows all loaded images by filename and provides:

- **Drag-and-drop reordering**: Drag an image entry up or down to change its display position. The layout updates immediately.
- **Image removal**: Click the `✖` button next to an image to remove it from the viewer. Remaining images are re-laid out automatically.

## Save Cropped Images

Press `P` or click the `⛶` icon in the bottom bar to save each image's currently visible crop to disk. Files are saved in JPEG format next to the original, with a `_crop` suffix (e.g., `photo.png` becomes `photo_crop.jpg`). This is useful for exporting exactly the region you're viewing.

## Review Mode

Review mode is designed for navigating through sets of related images that follow a naming convention (e.g., renders with different passes, image processing results with different parameters).

**How to use:**

1. Open 2 or more images with a common naming pattern. For example: `shot_001_diffuse.jpg` and `shot_001_specular.jpg`.
2. Click the **Review** toggle in the bottom bar.
3. The app analyzes the filenames, finds the common radix (`shot_001`), and determines the varying parts (`_diffuse.jpg`, `_specular.jpg`).
4. It scans the directory for all other radixes that match these patterns (e.g., `shot_002`, `shot_003`, ...).
5. Use `◀` / `▶` buttons to navigate through the matching sets.
6. The regex patterns for each cell are displayed and editable. Modify them and click `↻` to reload the directory with updated patterns.
7. Click `♲` to re-analyze patterns from the currently loaded images (useful after manually loading different files).

Review mode requires files to share a directory and follow a consistent naming structure.

## Settings Panel

Click the `⚙` icon to open the settings panel on the right side:

- **Scroll behavior**: Choose between None (disabled), Move (scroll pans), or Zoom (scroll zooms).
- **Interpolation**: Nearest-neighbor (sharp pixels, good for pixel art) or Bilinear (smooth, good for photos).
- **Multi cursor**: Toggle on/off.
- **Grid width**: Number of columns in grid layout. Set to 0 for automatic sizing.
- **Font size**: Adjust the size of the filename label displayed on each image.
- **Font color**: Pick the color for filename labels.
- **Keyboard shortcuts**: Click any shortcut button to enter rebinding mode, then press the desired key.
- **Save Settings**: Write current settings to `~/.image_viewer` so they persist across sessions.

## Interface Toggle

Press `H` to hide or show the entire UI (bottom bar, settings panel, image list). Useful for a distraction-free fullscreen view.

## Drag and Drop

Drag image files from your file explorer directly into the window. Multiple files can be dropped at once. On macOS, you can also drop images onto the app icon in the Dock or use "Open With" from Finder.

## HDR Support

HDR rendering can be enabled by setting `hdr.enabled = true` in the config file. This adds an HDR component to the camera for high dynamic range image viewing.

## Image Format Support

Supports most formats from the [image-rs](https://crates.io/crates/image) crate: JPEG, PNG, BMP, TIFF, EXR, GIF, WebP, and more. Images are uploaded as GPU textures, so they must be compatible with wgpu. Supported color types include RGB8, RGBA8, L8, LA8, RGB16, RGBA16, and L16.

## Configuration File

Settings are stored in a TOML file at `~/.image_viewer`. If the file doesn't exist, built-in defaults are used. The default configuration is documented in `assets/default/config.toml`. Settings changed through the UI can be persisted by clicking "Save Settings".
