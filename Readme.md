# Image Viewer

Simple image viewer that allow to compare image either side-by-side or by switching between them.

- Github : [https://github.com/edmBernard/image-viewer-rs](https://github.com/edmBernard/image-viewer-rs)

## Usage

### Open Images

- Drag images directly from file explorer. We can drag several images. It remove old images from the viewer.
- Pass image in the command line argument : `image-viewer-rs file1 file2 ...`

### Change layout

There are 3 layout available
- horizontal
- vertical
- stacked

### Change image on top (in Stacked Layout)

When images are stacked, it is possible to bring the image on top by using the 1, 2, ... keyboard keys.

## Image Format supported

We use the crate [image-rs](https://crates.io/crates/image), so be support almost the same number of format with some exception.
As image are uploaded as texture, the image should be compatible with [wgpu](https://crates.io/crates/wgpu) texture.
For example currently 16u images are supported by converting them 8u.

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

## Disclaimer

It's a toy project mainly used to learn bevy. So if you spot error, improvement comments are welcome.
