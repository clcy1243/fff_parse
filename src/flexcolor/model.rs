/// FlexColor edit history parser
/// Parses the XML plist stored in Imacon FFF tag 0xC519
use std::fmt;

use super::parser;

/// A single image setting (edit history entry)
#[derive(Debug, Clone)]
pub struct ImageSetting {
    pub name: String,
    pub info: String,
    pub flags: i64,
    pub created: DateTime,
    pub modified: DateTime,
    pub correction: ImageCorrection,
}

/// Date-time from the plist
#[derive(Debug, Clone, Default)]
pub struct DateTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

impl DateTime {
    pub fn is_valid(&self) -> bool {
        self.year > 0
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(
                f,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                self.year, self.month, self.day, self.hour, self.minute, self.second
            )
        } else {
            write!(f, "—")
        }
    }
}

/// Image correction parameters
#[derive(Debug, Clone, Default)]
pub struct ImageCorrection {
    pub contrast: i64,
    pub brightness: i64,
    pub gamma: f64,
    pub lightness: i64,
    pub saturation: i64,
    pub color_temperature: i64,
    pub tint: i64,
    pub ev: f64,
    pub film_curve: i64,
    pub film_type: i64,
    pub color_model: i64,
    pub apply_sliders: bool,
    pub apply_curves: bool,
    pub apply_histogram: bool,
    pub apply_usm: bool,
    pub apply_dust: bool,
    pub apply_cc: bool,
    pub apply_cn_filter: bool,
    pub usm_amount: i64,
    pub usm_radius: i64,
    pub usm_dark_limit: i64,
    pub usm_noise_limit: i64,
    pub threshold: i64,
    pub dust_level: i64,
    pub color_noise_radius: i64,
    pub noise_filter_bias: i64,
    pub lens_correction: i64,
    pub vignette_amount: i64,
    pub enhanced_shadow: bool,
    pub remove_cast_highlight: bool,
    pub remove_cast_shadow: bool,
    pub embed_profile: bool,
    pub convert: bool,
    pub soft_proof: bool,
    pub auto_highlight: i64,
    pub auto_shadow: i64,
    pub mode: i64,
    pub usm_col_factor: i64,
    /// Histogram levels: Shadow per channel [RGB, R, G, B]
    pub shadow: [i64; 4],
    /// Histogram levels: Gray (midtone) per channel [RGB, R, G, B]
    pub gray: [i64; 4],
    /// Histogram levels: Highlight per channel [RGB, R, G, B]
    pub highlight: [i64; 4],
    /// Color correction matrix: 36 values (channel × component)
    pub color_corr: Vec<i64>,
    /// Gradation sliders [Contrast, Brightness, ShadowDepth]
    pub gradation_sliders: [i64; 3],
    /// Gradation curve points per channel: [master, R, G, B, ...] each with [(x,y,dy), ...]
    pub gradations: Vec<Vec<(i64, i64, i64)>>,
    /// Input ICC profile name (e.g. "Flextight Input")
    pub input_profile_name: Option<String>,
    /// Output RGB profile name (e.g. "sRGB Color Space Profile.icm")
    pub rgb_profile_name: Option<String>,
    /// All raw key-value pairs (for display of unknown fields)
    pub raw_params: Vec<(String, String)>,
}

/// Parsed FlexColor edit history
#[derive(Debug, Clone)]
pub struct EditHistory {
    pub settings: Vec<ImageSetting>,
    pub current_index: usize,
}

impl EditHistory {
    /// Parse edit history from the raw bytes of tag 0xC519
    pub fn parse(data: &[u8]) -> Option<Self> {
        // Find the XML plist within the data
        let xml_start = find_subsequence(data, b"<?xml")?;
        let plist_end_marker = b"</plist>";
        let xml_end = find_subsequence(&data[xml_start..], plist_end_marker)?
            + xml_start
            + plist_end_marker.len();

        let xml_str = std::str::from_utf8(&data[xml_start..xml_end]).ok()?;
        Self::parse_xml(xml_str)
    }

    /// Parse from the full file data (searches for the XML plist)
    pub fn parse_from_file(data: &[u8]) -> Option<Self> {
        Self::parse(data)
    }

    fn parse_xml(xml: &str) -> Option<Self> {
        // Simple XML parser for Apple plist format
        let nodes = parser::parse_plist_nodes(xml)?;

        // The root should be: plist > dict > {key: ImageSettings, array: [...], key: CurrentIx, integer: N}
        let dict_node = parser::find_child_element(&nodes, "dict")?;

        let mut settings = Vec::new();
        let mut current_index = 0usize;

        let dict_children = parser::element_children(dict_node);
        let mut i = 0;
        while i + 1 < dict_children.len() {
            let key = parser::get_element_text(dict_children[i]);
            let val = dict_children[i + 1];

            match key.as_deref() {
                Some("ImageSettings") => {
                    if let Some(entries) = parser::parse_image_settings_array(val) {
                        settings = entries;
                    }
                }
                Some("CurrentIx") => {
                    if let Some(idx) = parser::get_element_text(val).and_then(|s| s.parse::<usize>().ok())
                    {
                        current_index = idx;
                    }
                }
                _ => {}
            }

            i += 2;
        }

        if settings.is_empty() {
            return None;
        }

        Some(EditHistory {
            settings,
            current_index,
        })
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Human-readable name for film curve type
pub fn film_curve_name(v: i64) -> &'static str {
    match v {
        0 => "Linear",
        1 => "Film Std",
        2 => "Film High",
        3 => "Film Low",
        4 => "Film Auto",
        _ => "Unknown",
    }
}

/// Human-readable name for film type
pub fn film_type_name(v: i64) -> &'static str {
    match v {
        0 => "Positive E-6",
        1 => "Negative C-41",
        2 => "B&W",
        _ => "Unknown",
    }
}

/// Human-readable name for color model
pub fn color_model_name(v: i64) -> &'static str {
    match v {
        0 => "RGB",
        1 => "CMYK",
        2 => "Grayscale",
        _ => "Unknown",
    }
}
