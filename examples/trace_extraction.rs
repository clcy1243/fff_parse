//! Trace the extraction reversal step-by-step for a specific pixel to find the bug.
use fff_viewer::flexcolor::EditHistory;
use fff_viewer::tiff::TiffFile;

use std::env;

fn invert_lut_256(forward: &[u8; 256]) -> [u8; 256] {
    let mut inv = [0u8; 256];
    for y in 0..256u16 {
        let mut best_x = 0u8;
        let mut best_dist = 256i32;
        for x in 0..256u16 {
            let dist = (forward[x as usize] as i32 - y as i32).abs();
            if dist < best_dist { best_dist = dist; best_x = x as u8; }
        }
        inv[y as usize] = best_x;
    }
    inv
}

fn invert_3x3(m: [[f32;3];3]) -> [[f32;3];3] {
    let det = m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1])
             -m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0])
             +m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0]);
    if det.abs() < 1e-10 { return [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]]; }
    let d = 1.0/det;
    [
        [(m[1][1]*m[2][2]-m[1][2]*m[2][1])*d,(m[0][2]*m[2][1]-m[0][1]*m[2][2])*d,(m[0][1]*m[1][2]-m[0][2]*m[1][1])*d],
        [(m[1][2]*m[2][0]-m[1][0]*m[2][2])*d,(m[0][0]*m[2][2]-m[0][2]*m[2][0])*d,(m[0][2]*m[1][0]-m[0][0]*m[1][2])*d],
        [(m[1][0]*m[2][1]-m[1][1]*m[2][0])*d,(m[0][1]*m[2][0]-m[0][0]*m[2][1])*d,(m[0][0]*m[1][1]-m[0][1]*m[1][0])*d],
    ]
}

fn main() {
    let path = env::args().nth(1).expect("Usage: trace <file.fff>");
    let data = std::fs::read(&path).unwrap();
    let tiff = TiffFile::parse(&data).unwrap();
    let hist = EditHistory::parse_from_file(tiff.raw_data()).expect("no edit history");
    let c = &hist.settings[hist.current_index].correction;
    
    let (thumb, preview) = tiff.decode_thumbnail_pair().expect("no thumbnail pair");
    let (w, h) = (thumb.width() as usize, thumb.height() as usize);
    
    println!("File: {}", path);
    println!("Correction: FilmType={}, Gamma={}", c.film_type, c.gamma);
    println!("Thumbnail: {}x{}", w, h);
    
    // Sample pixels at center and a few other locations
    let positions = [(w/2, h/2), (w/4, h/4), (3*w/4, 3*h/4), (w/3, h/3)];
    
    // Precompute parameters (matching extract_film_curve)
    let hi = [c.highlight[1] as f32*4.0, c.highlight[2] as f32*4.0, c.highlight[3] as f32*4.0];
    let scale = [if hi[0]>0.0{65535.0/hi[0]}else{1.0}, if hi[1]>0.0{65535.0/hi[1]}else{1.0}, if hi[2]>0.0{65535.0/hi[2]}else{1.0}];
    let mut bl=[0f32;3]; let mut wh_c=[0f32;3]; let mut gamma_c=[0f32;3];
    for ch in 0..3 { bl[ch]=c.shadow[ch+1]as f32*4.0/65535.0; wh_c[ch]=c.highlight[ch+1]as f32*4.0/65535.0; gamma_c[ch]=(c.gray[ch+1]as f32/128.0).max(0.01); }
    let gamma_m = ((c.gamma as f32)-1.0).max(0.01);
    let out_lo = if c.dot_color.len()>=14{[c.dot_color[0]as f32/255.0, c.dot_color[1]as f32/255.0, c.dot_color[2]as f32/255.0]}else{[0.0;3]};
    let out_hi = if c.dot_color.len()>=14{[c.dot_color[7]as f32/255.0, c.dot_color[8]as f32/255.0, c.dot_color[9]as f32/255.0]}else{[1.0;3]};
    let out_range = [(out_hi[0]-out_lo[0]).max(0.001), (out_hi[1]-out_lo[1]).max(0.001), (out_hi[2]-out_lo[2]).max(0.001)];
    let sat = if c.apply_sliders{c.saturation as f32/100.0}else{0.0};
    let contrast = if c.apply_sliders{c.contrast as f32/100.0}else{0.0};
    let brightness = if c.apply_sliders{c.brightness as f32/100.0}else{0.0};
    let lightness = if c.apply_sliders{c.lightness as f32/100.0}else{0.0};
    let apply_cc = c.apply_cc && c.color_corr.len()==36 && c.color_corr.iter().any(|&v|v!=0);
    let cc = if apply_cc {
        let m = &c.color_corr;
        [[(100+m[0])as f32/100.0, m[1]as f32/100.0, m[2]as f32/100.0],
         [m[6]as f32/100.0, (100+m[7])as f32/100.0, m[8]as f32/100.0],
         [m[12]as f32/100.0, m[13]as f32/100.0, (100+m[14])as f32/100.0]]
    } else { [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]] };
    let inv_cc = invert_3x3(cc);
    
    println!("\n=== Parameters ===");
    println!("sat={}, contrast={}, brightness={}, lightness={}", sat, contrast, brightness, lightness);
    println!("gamma_m={}, gamma_c={:?}", gamma_m, gamma_c);
    println!("bl={:?}, wh_c={:?}", bl, wh_c);
    println!("out_lo={:?}, out_hi={:?}, out_range={:?}", out_lo, out_hi, out_range);
    println!("apply_cc={}", apply_cc);
    if apply_cc {
        println!("CC matrix: {:?}", cc);
        println!("CC inverse: {:?}", inv_cc);
    }
    
    let has_grad = c.apply_curves && c.gradations.len()>=7 && !c.gradations.iter().all(|pts| pts.len()==2 && pts[0].0==0 && pts[0].1==0 && pts[1].0==255 && pts[1].1==255);
    println!("has_gradation_curves={}", has_grad);
    
    for &(px, py) in &positions {
        let pi = (py * w + px) * 3;
        let thumb_raw = thumb.as_raw();
        let prev_raw = preview.as_raw();
        
        // 16-bit raw → inverted
        let mut inv = [0f32;3];
        for ch in 0..3 { inv[ch] = ((hi[ch] - prev_raw[pi+ch] as f32).max(0.0) * scale[ch]).clamp(0.0, 65535.0); }
        if c.film_type==2 { let l=0.299*inv[0]+0.587*inv[1]+0.114*inv[2]; inv=[l,l,l]; }
        
        // Thumbnail values
        let mut rgb = [thumb_raw[pi]as f32/255.0, thumb_raw[pi+1]as f32/255.0, thumb_raw[pi+2]as f32/255.0];
        
        println!("\n--- Pixel ({},{}) ---", px, py);
        println!("  Raw16: ({},{},{})", prev_raw[pi], prev_raw[pi+1], prev_raw[pi+2]);
        println!("  Inverted: ({:.1},{:.1},{:.1})", inv[0], inv[1], inv[2]);
        println!("  Thumb8: ({},{},{})", thumb_raw[pi], thumb_raw[pi+1], thumb_raw[pi+2]);
        println!("  Step0 thumb_float: ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3a: inv(sat)
        if sat.abs()>0.001 {
            let lum = 0.2126*rgb[0]+0.7152*rgb[1]+0.0722*rgb[2];
            for ch in 0..3 { rgb[ch] = lum+(rgb[ch]-lum)/(1.0+sat); }
        }
        println!("  After inv(sat): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3b: inv(CC)
        if apply_cc {
            let (r0,g0,b0)=(rgb[0],rgb[1],rgb[2]);
            rgb[0]=(inv_cc[0][0]*r0+inv_cc[0][1]*g0+inv_cc[0][2]*b0).clamp(0.0,1.0);
            rgb[1]=(inv_cc[1][0]*r0+inv_cc[1][1]*g0+inv_cc[1][2]*b0).clamp(0.0,1.0);
            rgb[2]=(inv_cc[2][0]*r0+inv_cc[2][1]*g0+inv_cc[2][2]*b0).clamp(0.0,1.0);
        }
        println!("  After inv(CC): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3c: inv(lightness)
        if lightness.abs()>0.001 {
            let gamma = 1.0/(1.0+lightness).max(0.1);
            let inv_gamma = 1.0/gamma;
            for ch in 0..3 { rgb[ch] = rgb[ch].powf(inv_gamma).clamp(0.0,1.0); }
        }
        println!("  After inv(lightness): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3d: inv(brightness)
        if brightness.abs()>0.001 {
            for ch in 0..3 { rgb[ch] = (rgb[ch]-brightness*0.5).clamp(0.0,1.0); }
        }
        println!("  After inv(brightness): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3e: inv(contrast)
        if contrast.abs()>0.001 {
            let c_scale = if contrast>=0.0{1.0+contrast*2.0}else{1.0+contrast};
            let inv_scale = 1.0/c_scale.max(0.001);
            for ch in 0..3 { rgb[ch] = ((rgb[ch]-0.5)*inv_scale+0.5).clamp(0.0,1.0); }
        }
        println!("  After inv(contrast): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3f: inv(exposure)
        println!("  After inv(exposure): ({:.4},{:.4},{:.4}) [no change, EV=1]", rgb[0], rgb[1], rgb[2]);
        
        // 3g: inv(output_levels)
        for ch in 0..3 { rgb[ch] = ((rgb[ch]-out_lo[ch])/out_range[ch]).clamp(0.0,1.0); }
        println!("  After inv(output): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3h: inv(master_gamma)
        for ch in 0..3 { rgb[ch] = rgb[ch].powf(gamma_m); }
        println!("  After inv(master_gamma): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3i: inv(per-ch gamma)
        for ch in 0..3 { rgb[ch] = rgb[ch].powf(gamma_c[ch]); }
        println!("  After inv(per_gamma): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // 3j: inv(levels)
        for ch in 0..3 { let range=(wh_c[ch]-bl[ch]).max(0.001); rgb[ch] = (rgb[ch]*range+bl[ch]).clamp(0.0,1.0); }
        println!("  After inv(levels): ({:.4},{:.4},{:.4})", rgb[0], rgb[1], rgb[2]);
        
        // Bin index for this pixel
        for ch in 0..3 {
            let bin = ((inv[ch]/65535.0)*1023.0) as usize;
            println!("  Ch{}: inverted_bin={}, lut_value={:.4}", ch, bin.min(1023), rgb[ch]);
        }
    }
}
