use std::env;

use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::tags;
use fff_viewer::tiff::TiffFile;

fn main() {
    let path = env::args().nth(1).expect("Usage: parse_test <file.fff>");
    println!("Parsing: {}", path);

    match TiffFile::open(&path) {
        Ok(tiff_file) => {
            println!("\n=== File Info ===");
            println!("Byte order: {:?}", tiff_file.byte_order);
            println!("Magic: 0x{:04X}", tiff_file.magic);
            println!("IFD count: {}", tiff_file.ifds.len());
            println!(
                "Preview JPEG: {}",
                if tiff_file.preview_jpeg.is_some() {
                    format!(
                        "Yes ({} bytes)",
                        tiff_file.preview_jpeg.as_ref().unwrap().len()
                    )
                } else {
                    "No".to_string()
                }
            );

            println!("\n=== Metadata Summary ===");
            for (key, value) in tiff_file.metadata_summary() {
                println!("  {:>20}: {}", key, value);
            }

            println!("\n=== All IFDs ===");
            for ifd in &tiff_file.ifds {
                println!("\n--- {} ({} entries) ---", ifd.name, ifd.entries.len());
                for entry in ifd.entries.values() {
                    let tag_name = if ifd.name == "MakerNote" {
                        tags::makernote_tag_name(entry.tag)
                            .or_else(|| tags::standard_tag_name(entry.tag))
                            .unwrap_or("Unknown")
                    } else {
                        tags::standard_tag_name(entry.tag).unwrap_or("Unknown")
                    };
                    let val_str = entry.value.to_string();
                    let display = if val_str.len() > 80 {
                        format!("{}...", &val_str[..77])
                    } else {
                        val_str
                    };
                    println!("  0x{:04X} {:30} = {}", entry.tag, tag_name, display);
                }
            }

            // Test image decode
            println!("\n=== Image Decode Test ===");
            if let Some(img) = tiff_file.decode_preview_image() {
                println!("Decoded preview: {}x{}", img.width(), img.height());
            } else {
                println!("No decodable preview image");
            }

            // Test edit history
            println!("\n=== FlexColor Edit History ===");
            if let Some(history) = EditHistory::parse_from_file(tiff_file.raw_data()) {
                println!(
                    "{} settings, current index: {}",
                    history.settings.len(),
                    history.current_index
                );
                for (idx, setting) in history.settings.iter().enumerate() {
                    let marker = if idx == history.current_index {
                        "▶"
                    } else {
                        " "
                    };
                    println!(
                        "  {} [{}] \"{}\" — Created: {}",
                        marker, idx, setting.name, setting.created
                    );
                    let c = &setting.correction;
                    println!(
                        "        γ={} EV={} Contrast={} Brightness={} Saturation={}",
                        c.gamma, c.ev, c.contrast, c.brightness, c.saturation
                    );
                    println!(
                        "        Lightness={} ColorTemp={} Tint={}",
                        c.lightness, c.color_temperature, c.tint
                    );
                    println!(
                        "        FilmCurve={} FilmType={} ColorModel={}",
                        flexcolor::film_curve_name(c.film_curve),
                        flexcolor::film_type_name(c.film_type),
                        flexcolor::color_model_name(c.color_model)
                    );
                    println!(
                        "        Shadow={:?} Gray={:?} Highlight={:?}",
                        c.shadow, c.gray, c.highlight
                    );
                    if !c.dot_color.is_empty() {
                        println!("        DotColor={:?}", c.dot_color);
                    }
                    println!(
                        "        apply: sliders={} curves={} histogram={} usm={} dust={} cc={} cn_filter={}",
                        c.apply_sliders, c.apply_curves, c.apply_histogram,
                        c.apply_usm, c.apply_dust, c.apply_cc, c.apply_cn_filter
                    );
                    println!(
                        "        USM: amount={} radius={} dark_limit={} noise_limit={} col_factor={}",
                        c.usm_amount, c.usm_radius, c.usm_dark_limit, c.usm_noise_limit, c.usm_col_factor.len()
                    );
                    println!(
                        "        Dust: level={} threshold={} | LensCorr={} Vignette={}",
                        c.dust_level, c.threshold, c.lens_correction, c.vignette_amount
                    );
                    println!(
                        "        enhanced_shadow={} remove_cast_highlight={} remove_cast_shadow={}",
                        c.enhanced_shadow, c.remove_cast_highlight, c.remove_cast_shadow
                    );
                    println!(
                        "        input_profile={:?} rgb_profile={:?}",
                        c.input_profile_name, c.rgb_profile_name
                    );
                    println!(
                        "        auto_highlight={} auto_shadow={} mode={}",
                        c.auto_highlight, c.auto_shadow, c.mode
                    );
                    println!(
                        "        gradation_sliders={:?}",
                        c.gradation_sliders
                    );
                    println!(
                        "        color_corr ({} vals)={:?}",
                        c.color_corr.len(), c.color_corr
                    );
                    for (gi, curve) in c.gradations.iter().enumerate() {
                        if !curve.is_empty() {
                            let name = ["RGB","R","G","B","C","M","Y"].get(gi).unwrap_or(&"?");
                            println!("        gradation[{}]({}) {} pts: {:?}", gi, name, curve.len(), curve);
                        }
                    }
                }
            } else {
                println!("No edit history found");
            }
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
