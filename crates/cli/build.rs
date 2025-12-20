// On Windows, set the application icon
// On Linux/macOS, this build script does nothing
fn main() {
    #[cfg(target_os = "windows")]
    {
        let icon_path = "../../assets/hlbc.ico";
        println!("cargo:rerun-if-changed={icon_path}");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon_path);
        res.compile().unwrap();
    }
}
