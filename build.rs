fn main() {
    println!("cargo:rerun-if-changed=icon.png");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let ico_path = format!("{out_dir}/icon.ico");

    let src = image::open("icon.png").expect("icon.png not found in repo root");
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16u32, 32, 48, 256] {
        let resized = src.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        let img = ico::IconImage::from_rgba_data(size, size, rgba.into_raw());
        icon_dir.add_entry(ico::IconDirEntry::encode(&img).unwrap());
    }
    let f = std::fs::File::create(&ico_path).expect("failed to create icon.ico");
    icon_dir.write(f).expect("failed to write icon.ico");

    winresource::WindowsResource::new()
        .set_icon(&ico_path)
        .compile()
        .expect("failed to compile Windows resources");
}
