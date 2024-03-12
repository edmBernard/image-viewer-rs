# Image Viewer

Simple image viewer that allow to compare image either side-by-side or by switching between them.

- Github : [https://github.com/edmBernard/image-viewer-rs](https://github.com/edmBernard/image-viewer-rs)

## Usage

### Open Images

- Drag images directly from file explorer. We can drag several images. It remove old images from the viewer.
- Pass image in the command line argument : `image-viewer-rs file1 file2 ...`

### Change layout

There are 4 layouts available
- grid
- stacked
- vertical
- horizontal

We can change it by pressing `L` keyboard key to circle on them.

### Change image on top (in Stacked Layout)

When images are stacked, it is possible to bring the image on top by using the `1`, `2`, ... keyboard keys.

### Change zoom

There is 2 way to change the zoom :
- mouse scroll
- keyboard key: `Ctrl` + `1`,`2`,`3`,`4`,`5` set a fixed zoom of respectively 1,2,4,8,16

### Change Rotation

We can quarter turn image by pressing `R` keyboard key.

## Image Format supported

We use the crate [image-rs](https://crates.io/crates/image), so be support almost the same number of format with some exception.
As image are uploaded as texture, the image should be compatible with [wgpu](https://crates.io/crates/wgpu) texture.
For example currently 16u images are supported by converting them 8u.

## Config

There is a config file that allow to change shortcut, font and other things. you create a config file named `.image-viewer` at the root of the user directory. The default config is shown in [assets/default/config.toml](assets/default/config.toml).

## Build

### Get source

The recommended way to obtain the source code is to clone the entire repository from GitHub:

```
git clone git@github.com:edmBernard/image-viewer-rs
```

Building the main executable is done by the following command :

```bash
cargo build --release
```

The executable is named `image-viewer-rs`

### Bundle for macOS

Install [cargo-bundle](https://github.com/burtonageo/cargo-bundle). and run the following command :

```bash
cargo bundle
```

It will generate a `image-viewer.app`.

*Note*: Creating a bundle on osx allow to remove the execution of terminal in background of the GUI.

## Disclaimer

It's a toy project mainly used to learn bevy. So if you spot error, improvement comments are welcome.
