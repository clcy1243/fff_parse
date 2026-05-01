//! 测量切图路径各阶段耗时。用法：
//!   cargo run --release --example profile_load -- /path/to/file.fff

use fff_viewer::{color, flexcolor::EditHistory, tiff::TiffFile};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: profile_load <file.fff> [--iter N]");
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    let iter: usize = args
        .iter()
        .position(|a| a == "--iter")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("target: {} ({:.1} MB)", path.display(), file_size as f64 / 1024.0 / 1024.0);
    println!("iterations: {}", iter);
    println!();

    // 累加计时
    let mut tot = Timings::default();
    for i in 0..iter {
        let run = profile_once(&path);
        if iter <= 3 || i == 0 {
            println!("--- iter {} ---", i + 1);
            run.print();
        }
        tot.add(&run);
    }

    if iter > 1 {
        tot.divide(iter);
        println!("\n--- average over {} iter ---", iter);
        tot.print();
    }
}

#[derive(Default)]
struct Timings {
    fs_read: f64,
    parse: f64,
    metadata: f64,
    all_tags: f64,
    edit_history: f64,
    decode_preview: f64,
    film_process: f64,
    extract_icc: f64,
    sidecar_load: f64,
    total: f64,
}

impl Timings {
    fn add(&mut self, o: &Self) {
        self.fs_read += o.fs_read;
        self.parse += o.parse;
        self.metadata += o.metadata;
        self.all_tags += o.all_tags;
        self.edit_history += o.edit_history;
        self.decode_preview += o.decode_preview;
        self.film_process += o.film_process;
        self.extract_icc += o.extract_icc;
        self.sidecar_load += o.sidecar_load;
        self.total += o.total;
    }
    fn divide(&mut self, n: usize) {
        let n = n as f64;
        self.fs_read /= n;
        self.parse /= n;
        self.metadata /= n;
        self.all_tags /= n;
        self.edit_history /= n;
        self.decode_preview /= n;
        self.film_process /= n;
        self.extract_icc /= n;
        self.sidecar_load /= n;
        self.total /= n;
    }
    fn print(&self) {
        let rows = [
            ("fs::read (full file)", self.fs_read),
            ("TiffFile::parse", self.parse),
            ("metadata_summary", self.metadata),
            ("all_tags", self.all_tags),
            ("edit_history parse", self.edit_history),
            ("decode_preview_downscaled", self.decode_preview),
            ("apply_film_processing", self.film_process),
            ("extract_embedded_icc", self.extract_icc),
            ("sidecar::load", self.sidecar_load),
        ];
        for (label, ms) in rows {
            let pct = if self.total > 0.0 { ms / self.total * 100.0 } else { 0.0 };
            println!("  {:30} {:>8.1} ms  ({:>5.1}%)", label, ms, pct);
        }
        println!("  {:30} {:>8.1} ms  (100.0%)", "TOTAL", self.total);
    }
}

fn profile_once(path: &std::path::Path) -> Timings {
    let mut t = Timings::default();
    let t0 = Instant::now();

    let read_t = Instant::now();
    let data = std::fs::read(path).unwrap();
    t.fs_read = read_t.elapsed().as_secs_f64() * 1000.0;

    let parse_t = Instant::now();
    let tiff = TiffFile::parse(&data).unwrap();
    t.parse = parse_t.elapsed().as_secs_f64() * 1000.0;

    let md_t = Instant::now();
    let _metadata = tiff.metadata_summary();
    t.metadata = md_t.elapsed().as_secs_f64() * 1000.0;

    let tags_t = Instant::now();
    let all_tags = tiff.all_tags();
    t.all_tags = tags_t.elapsed().as_secs_f64() * 1000.0;

    let eh_t = Instant::now();
    let edit_history = EditHistory::parse_from_tiff(&tiff);
    t.edit_history = eh_t.elapsed().as_secs_f64() * 1000.0;

    let pv_t = Instant::now();
    let preview = tiff.decode_preview_downscaled(1600); // DISPLAY_MAX_DIM
    t.decode_preview = pv_t.elapsed().as_secs_f64() * 1000.0;

    let fp_t = Instant::now();
    if let (Some(img), Some(h)) = (preview.as_ref(), edit_history.as_ref()) {
        if !h.settings.is_empty() {
            let idx = h.current_index.min(h.settings.len() - 1);
            let corr = &h.settings[idx].correction;
            let _ = color::apply_film_processing(img, corr);
        }
    }
    t.film_process = fp_t.elapsed().as_secs_f64() * 1000.0;

    let icc_t = Instant::now();
    let _icc = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    t.extract_icc = icc_t.elapsed().as_secs_f64() * 1000.0;

    let sc_t = Instant::now();
    let _sc = fff_viewer::sidecar::load(path);
    t.sidecar_load = sc_t.elapsed().as_secs_f64() * 1000.0;

    t.total = t0.elapsed().as_secs_f64() * 1000.0;
    t
}
