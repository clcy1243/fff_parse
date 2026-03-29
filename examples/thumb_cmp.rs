use fff_viewer::tiff::TiffFile;
use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    for path in &args {
        let data = std::fs::read(path).unwrap();
        let tiff = TiffFile::parse(&data).unwrap();
        if let Some((thumb, _)) = tiff.decode_thumbnail_pair() {
            // Print first 20 pixel values and dimensions
            let raw = thumb.as_raw();
            println!("Thumb for {}: {}x{}", path, thumb.width(), thumb.height());
            print!("  First 30 bytes: ");
            for b in raw.iter().take(30) {
                print!("{} ", b);
            }
            println!();
            // Compute simple checksum
            let sum: u64 = raw.iter().map(|&b| b as u64).sum();
            println!("  Byte sum: {}", sum);
            // Corner pixel samples
            let w = thumb.width() as usize;
            let h = thumb.height() as usize;
            let mid = (h/2 * w + w/2) * 3;
            println!("  Center pixel: ({},{},{})", raw[mid], raw[mid+1], raw[mid+2]);
        }
    }
}
