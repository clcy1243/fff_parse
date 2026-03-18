//! Sidecar XML file: persists color settings and split regions per source file.
//!
//! For each source file, the sidecar is stored at
//! `~/fff_parse/sidecar/{sha256_of_absolute_path}.xml`.
//! The sidecar is never written back into the source file.

use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::color;
use crate::config;

/// All per-file settings that should persist across sessions.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    // Color settings
    pub use_embedded_correction: bool,
    pub use_embedded_icc: bool,
    pub input_profile_name: Option<String>,
    pub preset_name: Option<String>,
    pub target_color_space: String,

    // Split settings
    pub split_format: String,
    pub split_portrait: bool,
    pub split_naming_pattern: String,
    pub split_regions: Vec<SidecarRegion>,
    pub manual_adjust: color::ManualAdjust,
}

#[derive(Debug, Clone)]
pub struct SidecarRegion {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub angle: f32,
}

/// Compute the sidecar path: `~/fff_parse/sidecar/{hash}.xml`
/// where hash is derived from the source file's absolute path.
pub fn sidecar_path(source: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(source)
        .unwrap_or_else(|_| source.to_path_buf());
    let mut hasher = DefaultHasher::new();
    abs.to_string_lossy().as_ref().hash(&mut hasher);
    let hash = hasher.finish();
    config::sidecar_dir().join(format!("{:016x}.xml", hash))
}

/// Load a sidecar config from disk. Returns `None` if file doesn't exist or is invalid.
pub fn load(source: &Path) -> Option<SidecarConfig> {
    let path = sidecar_path(source);
    let xml = std::fs::read_to_string(&path).ok()?;
    parse_xml(&xml)
}

/// Save a sidecar config to disk in the sidecar directory.
pub fn save(source: &Path, config: &SidecarConfig) -> Result<(), String> {
    crate::config::ensure_dirs();
    let path = sidecar_path(source);
    let xml = to_xml(config);
    std::fs::write(&path, xml.as_bytes()).map_err(|e| format!("{}: {}", path.display(), e))
}

// ─── XML serialization ─────────────────────────────────────────────────────

fn to_xml(c: &SidecarConfig) -> String {
    let mut s = String::with_capacity(1024);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<fff_viewer>\n");

    // Color section
    s.push_str("  <color>\n");
    let _ = writeln!(s, "    <use_embedded_correction>{}</use_embedded_correction>", c.use_embedded_correction);
    let _ = writeln!(s, "    <use_embedded_icc>{}</use_embedded_icc>", c.use_embedded_icc);
    if let Some(ref name) = c.input_profile_name {
        let _ = writeln!(s, "    <input_profile>{}</input_profile>", xml_escape(name));
    }
    if let Some(ref name) = c.preset_name {
        let _ = writeln!(s, "    <preset>{}</preset>", xml_escape(name));
    }
    let _ = writeln!(s, "    <target_color_space>{}</target_color_space>", xml_escape(&c.target_color_space));
    s.push_str("  </color>\n");

    // Manual adjust section
    let a = &c.manual_adjust;
    s.push_str("  <adjust>\n");
    let _ = writeln!(s, "    <adj_enabled>{}</adj_enabled>", a.enabled);
    let _ = writeln!(s, "    <adj_exposure>{}</adj_exposure>", a.exposure);
    let _ = writeln!(s, "    <adj_contrast>{}</adj_contrast>", a.contrast);
    let _ = writeln!(s, "    <adj_highlights>{}</adj_highlights>", a.highlights);
    let _ = writeln!(s, "    <adj_shadows>{}</adj_shadows>", a.shadows);
    let _ = writeln!(s, "    <adj_saturation>{}</adj_saturation>", a.saturation);
    let _ = writeln!(s, "    <adj_r_shift>{}</adj_r_shift>", a.r_shift);
    let _ = writeln!(s, "    <adj_g_shift>{}</adj_g_shift>", a.g_shift);
    let _ = writeln!(s, "    <adj_b_shift>{}</adj_b_shift>", a.b_shift);
    s.push_str("  </adjust>\n");

    // Split section
    s.push_str("  <split>\n");
    let _ = writeln!(s, "    <format>{}</format>", xml_escape(&c.split_format));
    let _ = writeln!(s, "    <portrait>{}</portrait>", c.split_portrait);
    let _ = writeln!(s, "    <naming_pattern>{}</naming_pattern>", xml_escape(&c.split_naming_pattern));
    if !c.split_regions.is_empty() {
        s.push_str("    <regions>\n");
        for r in &c.split_regions {
            let _ = writeln!(
                s,
                "      <region cx=\"{:.6}\" cy=\"{:.6}\" w=\"{:.6}\" h=\"{:.6}\" angle=\"{:.6}\"/>",
                r.cx, r.cy, r.w, r.h, r.angle
            );
        }
        s.push_str("    </regions>\n");
    }
    s.push_str("  </split>\n");

    s.push_str("</fff_viewer>\n");
    s
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

// ─── XML parsing ────────────────────────────────────────────────────────────

fn parse_xml(xml: &str) -> Option<SidecarConfig> {
    // Validate root element
    if !xml.contains("<fff_viewer>") {
        return None;
    }

    let mut config = SidecarConfig {
        use_embedded_correction: false,
        use_embedded_icc: false,
        input_profile_name: None,
        preset_name: None,
        target_color_space: "ProPhotoRGB".to_string(),
        split_format: "Free".to_string(),
        split_portrait: false,
        split_naming_pattern: "{name}_{n}".to_string(),
        split_regions: Vec::new(),
        manual_adjust: color::ManualAdjust::default(),
    };

    // Parse simple tags
    if let Some(v) = tag_content(xml, "use_embedded_correction") {
        config.use_embedded_correction = v == "true";
    }
    if let Some(v) = tag_content(xml, "use_embedded_icc") {
        config.use_embedded_icc = v == "true";
    }
    if let Some(v) = tag_content(xml, "input_profile") {
        config.input_profile_name = Some(xml_unescape(&v));
    }
    if let Some(v) = tag_content(xml, "preset") {
        config.preset_name = Some(xml_unescape(&v));
    }
    if let Some(v) = tag_content(xml, "target_color_space") {
        config.target_color_space = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "format") {
        config.split_format = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "portrait") {
        config.split_portrait = v == "true";
    }
    if let Some(v) = tag_content(xml, "naming_pattern") {
        config.split_naming_pattern = xml_unescape(&v);
    }

    if let Some(v) = tag_content(xml, "adj_enabled") {
        config.manual_adjust.enabled = v == "true";
    }
    if let Some(v) = tag_content(xml, "adj_exposure") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.exposure = f; }
    }
    if let Some(v) = tag_content(xml, "adj_contrast") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.contrast = f; }
    }
    if let Some(v) = tag_content(xml, "adj_highlights") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.highlights = f; }
    }
    if let Some(v) = tag_content(xml, "adj_shadows") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.shadows = f; }
    }
    if let Some(v) = tag_content(xml, "adj_saturation") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.saturation = f; }
    }
    if let Some(v) = tag_content(xml, "adj_r_shift") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.r_shift = f; }
    }
    if let Some(v) = tag_content(xml, "adj_g_shift") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.g_shift = f; }
    }
    if let Some(v) = tag_content(xml, "adj_b_shift") {
        if let Ok(f) = v.parse::<f32>() { config.manual_adjust.b_shift = f; }
    }

    // Parse region elements: <region cx="..." cy="..." w="..." h="..." angle="..."/>
    let mut search_from = 0;
    while let Some(start) = xml[search_from..].find("<region ") {
        let abs_start = search_from + start;
        let tag_end = match xml[abs_start..].find("/>") {
            Some(e) => abs_start + e + 2,
            None => break,
        };
        let tag = &xml[abs_start..tag_end];
        if let Some(region) = parse_region_tag(tag) {
            config.split_regions.push(region);
        }
        search_from = tag_end;
    }

    Some(config)
}

/// Extract text content between `<tag>` and `</tag>`.
fn tag_content<'a>(xml: &'a str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].trim().to_string())
}

/// Parse a self-closing `<region ... />` tag.
fn parse_region_tag(tag: &str) -> Option<SidecarRegion> {
    Some(SidecarRegion {
        cx: attr_f32(tag, "cx")?,
        cy: attr_f32(tag, "cy")?,
        w: attr_f32(tag, "w")?,
        h: attr_f32(tag, "h")?,
        angle: attr_f32(tag, "angle").unwrap_or(0.0),
    })
}

/// Extract a float attribute value: `name="1.234"`.
fn attr_f32(tag: &str, name: &str) -> Option<f32> {
    let pattern = format!("{}=\"", name);
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')?;
    tag[start..start + end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let config = SidecarConfig {
            use_embedded_correction: true,
            use_embedded_icc: false,
            input_profile_name: Some("Scanner RGB".to_string()),
            preset_name: None,
            target_color_space: "AdobeRGB".to_string(),
            split_format: "Full35mm".to_string(),
            split_portrait: true,
            split_naming_pattern: "{name}_{n}".to_string(),
            split_regions: vec![
                SidecarRegion { cx: 0.5, cy: 0.25, w: 0.8, h: 0.3, angle: 0.05 },
                SidecarRegion { cx: 0.5, cy: 0.75, w: 0.8, h: 0.3, angle: -0.02 },
            ],
            manual_adjust: color::ManualAdjust::default(),
        };

        let xml = to_xml(&config);
        let parsed = parse_xml(&xml).unwrap();

        assert_eq!(parsed.use_embedded_correction, true);
        assert_eq!(parsed.use_embedded_icc, false);
        assert_eq!(parsed.input_profile_name.as_deref(), Some("Scanner RGB"));
        assert_eq!(parsed.preset_name, None);
        assert_eq!(parsed.target_color_space, "AdobeRGB");
        assert_eq!(parsed.split_format, "Full35mm");
        assert_eq!(parsed.split_portrait, true);
        assert_eq!(parsed.split_regions.len(), 2);
        assert!((parsed.split_regions[0].cx - 0.5).abs() < 0.001);
        assert!((parsed.split_regions[1].angle - (-0.02)).abs() < 0.001);
        assert!((parsed.manual_adjust.exposure - 0.0).abs() < 0.001);
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(load(Path::new("/nonexistent/file.fff")).is_none());
    }

    #[test]
    fn xml_escape_roundtrip() {
        let config = SidecarConfig {
            use_embedded_correction: false,
            use_embedded_icc: false,
            input_profile_name: Some("Profile <special> & \"quoted\"".to_string()),
            preset_name: None,
            target_color_space: "sRGB".to_string(),
            split_format: "Free".to_string(),
            split_portrait: false,
            split_naming_pattern: "{name}_{n}".to_string(),
            split_regions: vec![],
            manual_adjust: color::ManualAdjust::default(),
        };
        let xml = to_xml(&config);
        let parsed = parse_xml(&xml).unwrap();
        assert_eq!(
            parsed.input_profile_name.as_deref(),
            Some("Profile <special> & \"quoted\"")
        );
    }
}
