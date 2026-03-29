use fff_viewer::flexcolor::EditHistory;
use fff_viewer::tiff::TiffFile;
use std::env;

fn main() {
    let path = env::args().nth(1).expect("Usage: diag <file.fff>");
    let data = std::fs::read(&path).unwrap();
    let tiff = TiffFile::parse(&data).unwrap();
    
    if let Some(hist) = EditHistory::parse_from_file(tiff.raw_data()) {
        println!("File: {}", path);
        println!("Current index: {}", hist.current_index);
        println!("Total entries: {}", hist.settings.len());
        println!();
        
        for (i, s) in hist.settings.iter().enumerate() {
            let c = &s.correction;
            println!("=== Entry [{}] {} ===", i, if i == hist.current_index { "(CURRENT)" } else { "" });
            println!("  FilmType={}, Gamma={}, FilmCurve={}", c.film_type, c.gamma, c.film_curve);
            println!("  Shadow={:?}", c.shadow);
            println!("  Highlight={:?}", c.highlight);
            println!("  Gray={:?}", c.gray);
            println!("  EV={}, Saturation={}", c.ev, c.saturation);
            println!("  Contrast={}, Brightness={}, Lightness={}", c.contrast, c.brightness, c.lightness);
            println!("  apply_sliders={}, apply_curves={}, apply_cc={}, apply_histogram={}",
                c.apply_sliders, c.apply_curves, c.apply_cc, c.apply_histogram);
            let dc_len = std::cmp::min(14, c.dot_color.len());
            println!("  DotColor={:?}", &c.dot_color[..dc_len]);
            let cc_len = std::cmp::min(15, c.color_corr.len());
            println!("  ColorCorr (first 15)={:?}", &c.color_corr[..cc_len]);
            let grad_summary: Vec<usize> = c.gradations.iter().map(|g: &Vec<_>| g.len()).collect();
            println!("  Gradations points per channel: {:?}", grad_summary);
            println!();
        }
    }
}

// Also check thumbnail hash
fn thumb_hash() {
    for path_str in env::args().skip(1) {
        let data = std::fs::read(&path_str).unwrap();
        let tiff = TiffFile::parse(&data).unwrap();
        if let Some((thumb, _)) = tiff.decode_thumbnail_pair() {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            thumb.as_raw().hash(&mut hasher);
            let hash = hasher.finish();
            println!("Thumb hash for {}: {:016x} ({}x{}, {} bytes)", 
                path_str, hash, thumb.width(), thumb.height(), thumb.as_raw().len());
        } else {
            println!("No thumbnail for {}", path_str);
        }
    }
}
