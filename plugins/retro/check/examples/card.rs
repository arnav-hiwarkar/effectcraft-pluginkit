fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, width, height] = args.as_slice() else {
        eprintln!("usage: card <out.png> <width> <height>");
        std::process::exit(2);
    };
    let (w, h): (u32, u32) = (width.parse().expect("width"), height.parse().expect("height"));
    let bytes: Vec<u8> = retro_check::test_card(w, h).iter().flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    image::save_buffer(out, &bytes, w, h, image::ExtendedColorType::Rgba8).expect("write the card");
}
