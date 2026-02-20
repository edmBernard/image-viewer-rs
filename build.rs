fn main() {
    #[cfg(target_os = "windows")]
    winresource::WindowsResource::new()
        .set_icon("assets/icons/icon.ico")
        .compile()
        .expect("Failed to compile Windows resources");
}
