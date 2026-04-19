use lcms2::*;
fn main() {
    let path = std::env::args().nth(1).expect("pass icc");
    let data = std::fs::read(&path).expect("read");
    let p = Profile::new_icc(&data).expect("parse");
    println!("{} ({} bytes)", path, data.len());
    println!("Desc: {}", p.info(InfoType::Description, Locale::none()).unwrap_or_default());
    println!("DeviceClass: {:?}  ColorSpace: {:?}  PCS: {:?}", p.device_class(), p.color_space(), p.pcs());
    for sig in p.tag_signatures() {
        let v = sig as u32;
        let b = v.to_be_bytes();
        let s: String = b.iter().map(|&c| if c.is_ascii_graphic() { c as char } else { '?' }).collect();
        println!("  tag: {}", s);
    }
}
