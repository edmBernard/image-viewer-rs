use std::env;
use std::path::Path;
use std::fs;

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = &env::var("CARGO_MANIFEST_DIR").unwrap();
    let source_dir = Path::new(manifest);
    let config = env::var("PROFILE").unwrap();
    let target_dir = source_dir.join("target").join(config);

    let font_source = source_dir.join("assets").join("fonts").join("IBMPlexMono-Regular.otf");
    println!("FONTS_SOURCE={:?}", font_source);
    let font_dst_folder = target_dir.join("assets").join("fonts");
    println!("FONTS_DST_FOLDER={:?}", font_dst_folder);
    fs::create_dir_all(&font_dst_folder)?;
    let font_dst = font_dst_folder.join("IBMPlexMono-Regular.otf");
    println!("FONTS_DST={:?}", font_dst);
    fs::copy(font_source, font_dst)?;

    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}
