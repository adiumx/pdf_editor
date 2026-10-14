//! `cargo run --example fontinfo -- font.otf`: tables and a few Latin letters.
use ttf_parser::Face;
fn main() {
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).unwrap();
        let Ok(face) = Face::parse(&data, 0) else { println!("{path}: not sfnt"); continue };
        let cff = face.tables().cff.is_some();
        let glyf = face.tables().glyf.is_some();
        let has = |c: char| face.glyph_index(c).is_some();
        println!("{path}: cff={cff} glyf={glyf} glyphs={} ñ={} é={} Ñ={} Ž={} €={} A={} z={}", face.number_of_glyphs(), has('ñ'), has('é'), has('Ñ'), has('Ž'), has('€'), has('A'), has('z'));
    }
}
