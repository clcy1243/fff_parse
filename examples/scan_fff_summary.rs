use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::tiff::TiffFile;

fn collect_fff_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_fff_files(&path, out);
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("fff"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn main() {
    let root = env::args().nth(1).expect("Usage: cargo run --release --example scan_fff_summary -- <root>");
    let mut files = Vec::new();
    collect_fff_files(Path::new(&root), &mut files);
    files.sort();

    println!("path\twidth\theight\tentries\tcurrent_idx\tfilm_type\tcolor_model\tgamma\tinput_profile\trgb_profile\tsetting_name");
    for path in files {
        let Ok(tiff) = TiffFile::open(&path) else {
            eprintln!("WARN\topen\t{}", path.display());
            continue;
        };
        let width = tiff.ifds.get(0).and_then(|ifd| ifd.get_u32(0x0100)).unwrap_or(0);
        let height = tiff.ifds.get(0).and_then(|ifd| ifd.get_u32(0x0101)).unwrap_or(0);
        let Some(hist) = EditHistory::parse_from_tiff(&tiff) else {
            eprintln!("WARN\tedithistory\t{}", path.display());
            continue;
        };
        if hist.settings.is_empty() {
            eprintln!("WARN\tempty\t{}", path.display());
            continue;
        }
        let idx = hist.current_index.min(hist.settings.len() - 1);
        let setting = &hist.settings[idx];
        let corr = &setting.correction;
        let input_profile = corr.input_profile_name.as_deref().unwrap_or("");
        let rgb_profile = corr.rgb_profile_name.as_deref().unwrap_or("");
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{}\t{}\t{}",
            path.display(),
            width,
            height,
            hist.settings.len(),
            idx,
            flexcolor::film_type_name(corr.film_type),
            flexcolor::color_model_name(corr.color_model),
            corr.gamma,
            input_profile,
            rgb_profile,
            setting.name.replace('\t', " ")
        );
    }
}
