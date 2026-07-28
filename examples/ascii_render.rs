fn main() {
    let bytes = std::fs::read(r"C:\Windows\Fonts\arial.ttf").expect("arial.ttf should exist");
    let font = rusty_font::Font::parse(&bytes).expect("should parse");

    for ch in ['A', 'O', 'I', 'M', 'S'] {
        let glyph_id = font.glyph_index(ch).unwrap();
        let outline = font.glyph_outline(glyph_id).unwrap();
        let size = 32;
        let scale = size as f32 / font.units_per_em() as f32;
        let rasterizer = rusty_font::Rasterizer::new(size, size);
        let buffer = rasterizer.rasterize(&outline, scale);

        println!("--- '{ch}' ---");
        for y in (0..size).rev() {
            let mut line = String::new();
            for x in 0..size {
                line.push(if buffer[y * size + x] == 255 { '#' } else { '.' });
            }
            println!("{line}");
        }
        println!();
    }
}
