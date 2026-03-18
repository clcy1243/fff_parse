/// FlexColor edit history parser
/// Parses the XML plist stored in Imacon FFF tag 0xC519
use std::fmt;

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
        let nodes = parse_plist_nodes(xml)?;

        // The root should be: plist > dict > {key: ImageSettings, array: [...], key: CurrentIx, integer: N}
        let dict_node = find_child_element(&nodes, "dict")?;

        let mut settings = Vec::new();
        let mut current_index = 0usize;

        let dict_children = element_children(dict_node);
        let mut i = 0;
        while i + 1 < dict_children.len() {
            let key = get_element_text(dict_children[i]);
            let val = dict_children[i + 1];

            match key.as_deref() {
                Some("ImageSettings") => {
                    if let Some(entries) = parse_image_settings_array(val) {
                        settings = entries;
                    }
                }
                Some("CurrentIx") => {
                    if let Some(idx) = get_element_text(val).and_then(|s| s.parse::<usize>().ok())
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

// ===== Simple XML plist parser =====
// We use a minimal approach since these plists have a predictable structure.

#[derive(Debug, Clone)]
enum XmlNode {
    Element {
        name: String,
        children: Vec<XmlNode>,
    },
    Text(String),
}

fn parse_plist_nodes(xml: &str) -> Option<Vec<XmlNode>> {
    // Strip XML declaration and DOCTYPE
    let body = xml
        .find("<plist")
        .map(|i| &xml[i..])
        .unwrap_or(xml);

    let (nodes, _) = parse_xml_fragment(body);
    Some(nodes)
}

/// Recursive XML parser that returns (nodes, bytes_consumed).
/// Stops when it encounters a closing tag `</...>`, leaving it for the parent to consume.
fn parse_xml_fragment(s: &str) -> (Vec<XmlNode>, usize) {
    let mut nodes = Vec::new();
    let mut pos = 0;
    let bytes = s.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            // Closing tag — stop here; let the parent handle it
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
                break;
            }
            // Comment / processing instruction / DOCTYPE — skip
            if pos + 1 < bytes.len() && (bytes[pos + 1] == b'?' || bytes[pos + 1] == b'!') {
                if let Some(end) = s[pos..].find('>') {
                    pos += end + 1;
                    continue;
                }
                break;
            }

            // Parse opening tag
            let tag_end = match s[pos..].find('>') {
                Some(e) => pos + e,
                None => break,
            };

            let tag_content = &s[pos + 1..tag_end];

            // Self-closing tag like <true/> or <false/>
            if tag_content.ends_with('/') {
                let name = tag_content.trim_end_matches('/').trim().to_string();
                nodes.push(XmlNode::Element {
                    name,
                    children: vec![],
                });
                pos = tag_end + 1;
                continue;
            }

            let name = tag_content
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();

            // Content starts right after the opening tag '>'
            let content_start = tag_end + 1;
            let (children, consumed) = parse_xml_fragment(&s[content_start..]);
            let content_end = content_start + consumed;

            // Expect the matching closing tag at content_end
            let close_tag = format!("</{}>", name);
            if s[content_end..].starts_with(&close_tag) {
                let inner = &s[content_start..content_end];
                let final_children = if children.is_empty() && !inner.contains('<') {
                    let text = inner.trim();
                    if text.is_empty() {
                        vec![]
                    } else {
                        vec![XmlNode::Text(decode_xml_entities(text))]
                    }
                } else {
                    children
                };

                nodes.push(XmlNode::Element {
                    name,
                    children: final_children,
                });

                pos = content_end + close_tag.len();
            } else {
                // Malformed XML — save what we have and stop
                nodes.push(XmlNode::Element {
                    name,
                    children,
                });
                pos = content_end;
            }
        } else {
            pos += 1;
        }
    }

    (nodes, pos)
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn find_child_element<'a>(nodes: &'a [XmlNode], name: &str) -> Option<&'a XmlNode> {
    for node in nodes {
        match node {
            XmlNode::Element {
                name: n,
                children,
            } => {
                if n == name {
                    return Some(node);
                }
                // Search children too
                if let Some(found) = find_child_element(children, name) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn element_children(node: &XmlNode) -> Vec<&XmlNode> {
    match node {
        XmlNode::Element { children, .. } => children.iter().collect(),
        _ => vec![],
    }
}

fn get_element_text(node: &XmlNode) -> Option<String> {
    match node {
        XmlNode::Element { name, children } => {
            // For <true/> and <false/>, the tag name IS the value
            if name == "true" || name == "false" {
                return Some(name.clone());
            }
            // Get text from children
            for child in children {
                if let XmlNode::Text(t) = child {
                    return Some(t.clone());
                }
            }
            // Empty element
            Some(String::new())
        }
        XmlNode::Text(t) => Some(t.clone()),
    }
}

fn get_element_name(node: &XmlNode) -> Option<&str> {
    match node {
        XmlNode::Element { name, .. } => Some(name),
        _ => None,
    }
}

/// Extract the "Name" string from a profile dict node (InputProfile/RGBProfile).
/// These are stored as: <dict><key>Name</key><string>...</string>...</dict>
fn get_profile_name_from_dict(dict_node: &XmlNode) -> Option<String> {
    if get_element_name(dict_node) != Some("dict") {
        return None;
    }
    let children = element_children(dict_node);
    let mut i = 0;
    while i + 1 < children.len() {
        if get_element_text(children[i]).as_deref() == Some("Name") {
            return get_element_text(children[i + 1]);
        }
        i += 2;
    }
    None
}

fn parse_image_settings_array(array_node: &XmlNode) -> Option<Vec<ImageSetting>> {
    let children = element_children(array_node);
    let mut settings = Vec::new();

    for child in children {
        if get_element_name(child) == Some("dict") {
            if let Some(setting) = parse_single_setting(child) {
                settings.push(setting);
            }
        }
    }

    Some(settings)
}

fn parse_single_setting(dict_node: &XmlNode) -> Option<ImageSetting> {
    let children = element_children(dict_node);
    let mut name = String::new();
    let mut info = String::new();
    let mut flags = 0i64;
    let mut created = DateTime::default();
    let mut modified = DateTime::default();
    let mut correction = ImageCorrection::default();

    let mut i = 0;
    while i + 1 < children.len() {
        let key = get_element_text(children[i]).unwrap_or_default();
        let val = children[i + 1];

        match key.as_str() {
            "Name" => name = get_element_text(val).unwrap_or_default(),
            "Info" => info = get_element_text(val).unwrap_or_default(),
            "Flags" => flags = get_element_text(val).and_then(|s| s.parse().ok()).unwrap_or(0),
            "Created" => created = parse_datetime_dict(val),
            "Modified" => modified = parse_datetime_dict(val),
            "ImageCorrection" => correction = parse_image_correction(val),
            _ => {}
        }

        i += 2;
    }

    Some(ImageSetting {
        name,
        info,
        flags,
        created,
        modified,
        correction,
    })
}

fn parse_datetime_dict(dict_node: &XmlNode) -> DateTime {
    let children = element_children(dict_node);
    let mut dt = DateTime::default();

    let mut i = 0;
    while i + 1 < children.len() {
        let key = get_element_text(children[i]).unwrap_or_default();
        let val: i32 = get_element_text(children[i + 1])
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        match key.as_str() {
            "Year" => dt.year = val,
            "Month" => dt.month = val,
            "Day" => dt.day = val,
            "Hour" => dt.hour = val,
            "Minute" => dt.minute = val,
            "Second" => dt.second = val,
            _ => {}
        }

        i += 2;
    }

    dt
}

fn parse_image_correction(dict_node: &XmlNode) -> ImageCorrection {
    let children = element_children(dict_node);
    let mut corr = ImageCorrection::default();
    corr.gamma = 2.0; // default

    let mut i = 0;
    while i + 1 < children.len() {
        let key = get_element_text(children[i]).unwrap_or_default();
        let val = children[i + 1];
        let val_text = get_element_text(val).unwrap_or_default();
        let val_name = get_element_name(val).unwrap_or("");

        let int_val = || val_text.parse::<i64>().unwrap_or(0);
        let float_val = || val_text.parse::<f64>().unwrap_or(0.0);
        let bool_val = || val_name == "true" || val_text == "true";

        // Store raw param for display
        let display_val = match val_name {
            "true" => "true".to_string(),
            "false" => "false".to_string(),
            "array" | "dict" => "[complex]".to_string(),
            _ => val_text.clone(),
        };
        if key != "streamableVersion" && key != "Gradations" {
            corr.raw_params.push((key.clone(), display_val));
        }

        match key.as_str() {
            "Contrast" => corr.contrast = int_val(),
            "Brightness" => corr.brightness = int_val(),
            "Gamma" => corr.gamma = float_val(),
            "Lightness" => corr.lightness = int_val(),
            "Saturation" => corr.saturation = int_val(),
            "ColorTemperature" => corr.color_temperature = int_val(),
            "Tint" => corr.tint = int_val(),
            "EV" => corr.ev = float_val(),
            "FilmCurve" => corr.film_curve = int_val(),
            "FilmType" => corr.film_type = int_val(),
            "ColorModel" => corr.color_model = int_val(),
            "ApplySliders" => corr.apply_sliders = bool_val(),
            "ApplyCurves" => corr.apply_curves = bool_val(),
            "ApplyHistogram" => corr.apply_histogram = bool_val(),
            "ApplyUSM" => corr.apply_usm = bool_val(),
            "ApplyDust" => corr.apply_dust = bool_val(),
            "ApplyCC" => corr.apply_cc = bool_val(),
            "ApplyCNFilter" => corr.apply_cn_filter = bool_val(),
            "USMAmount" => corr.usm_amount = int_val(),
            "USMRadius" => corr.usm_radius = int_val(),
            "USMDarkLimit" => corr.usm_dark_limit = int_val(),
            "USMNoiseLimit" => corr.usm_noise_limit = int_val(),
            "USMColFactor" => corr.usm_col_factor = int_val(),
            "Threshold" => corr.threshold = int_val(),
            "DustLevel" => corr.dust_level = int_val(),
            "ColorNoiseRadius" => corr.color_noise_radius = int_val(),
            "NoiseFilterBias" => corr.noise_filter_bias = int_val(),
            "LensCorrection" => corr.lens_correction = int_val(),
            "VignetteAmount" => corr.vignette_amount = int_val(),
            "EnhancedShadow" => corr.enhanced_shadow = bool_val(),
            "RemoveCastHighlight" => corr.remove_cast_highlight = bool_val(),
            "RemoveCastShadow" => corr.remove_cast_shadow = bool_val(),
            "EmbedProfile" => corr.embed_profile = bool_val(),
            "Convert" => corr.convert = bool_val(),
            "SoftProof" => corr.soft_proof = bool_val(),
            "AutoHighlight" => corr.auto_highlight = int_val(),
            "AutoShadow" => corr.auto_shadow = int_val(),
            "Mode" => corr.mode = int_val(),
            "Gradations" => {
                corr.gradations = parse_gradations(val);
            }
            "Shadow" => {
                corr.shadow = parse_int_array_4(val);
            }
            "Gray" => {
                corr.gray = parse_int_array_4(val);
            }
            "Highlight" => {
                corr.highlight = parse_int_array_4(val);
            }
            "ColorCorr" => {
                corr.color_corr = parse_int_array(val);
            }
            "GradationSliders" => {
                let arr = parse_int_array(val);
                if arr.len() >= 3 {
                    corr.gradation_sliders = [arr[0], arr[1], arr[2]];
                }
            }
            "InputProfile" => {
                // InputProfile is a dict with a "Name" key
                if let Some(name) = get_profile_name_from_dict(val) {
                    corr.input_profile_name = Some(name);
                }
            }
            "RGBProfile" => {
                if let Some(name) = get_profile_name_from_dict(val) {
                    corr.rgb_profile_name = Some(name);
                }
            }
            _ => {}
        }

        i += 2;
    }

    corr
}

fn parse_gradations(array_node: &XmlNode) -> Vec<Vec<(i64, i64, i64)>> {
    let mut channels = Vec::new();
    for channel_dict in element_children(array_node) {
        if get_element_name(channel_dict) != Some("dict") {
            continue;
        }
        let dict_children = element_children(channel_dict);
        let mut i = 0;
        while i + 1 < dict_children.len() {
            let key = get_element_text(dict_children[i]).unwrap_or_default();
            if key == "Points" {
                let points = parse_gradation_points(dict_children[i + 1]);
                channels.push(points);
            }
            i += 2;
        }
    }
    channels
}

fn parse_gradation_points(array_node: &XmlNode) -> Vec<(i64, i64, i64)> {
    let mut points = Vec::new();
    for point_dict in element_children(array_node) {
        if get_element_name(point_dict) != Some("dict") {
            continue;
        }
        let children = element_children(point_dict);
        let mut x = 0i64;
        let mut y = 0i64;
        let mut dy = 1i64;
        let mut i = 0;
        while i + 1 < children.len() {
            let key = get_element_text(children[i]).unwrap_or_default();
            let val: i64 = get_element_text(children[i + 1])
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            match key.as_str() {
                "X" => x = val,
                "Y" => y = val,
                "DY" => dy = val,
                _ => {}
            }
            i += 2;
        }
        points.push((x, y, dy));
    }
    points
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Parse an <array> of <integer> values into a Vec<i64>
fn parse_int_array(array_node: &XmlNode) -> Vec<i64> {
    element_children(array_node)
        .iter()
        .filter_map(|n| get_element_text(n).and_then(|s| s.parse::<i64>().ok()))
        .collect()
}

/// Parse an <array> of 4 integers into [i64; 4] (for Shadow/Gray/Highlight)
fn parse_int_array_4(array_node: &XmlNode) -> [i64; 4] {
    let vals = parse_int_array(array_node);
    [
        vals.first().copied().unwrap_or(0),
        vals.get(1).copied().unwrap_or(0),
        vals.get(2).copied().unwrap_or(0),
        vals.get(3).copied().unwrap_or(0),
    ]
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

/// Parse a FlexColor settings XML preset file and extract ImageCorrection.
/// These files have the same plist format as embedded edit history,
/// with structure: dict > ImageSetting > dict > ImageCorrection > dict
pub fn parse_settings_xml(xml_str: &str) -> Option<ImageCorrection> {
    let nodes = parse_plist_nodes(xml_str)?;
    let root_dict = find_child_element(&nodes, "dict")?;
    let root_children = element_children(root_dict);

    let mut i = 0;
    while i + 1 < root_children.len() {
        let key = get_element_text(root_children[i]).unwrap_or_default();
        let val = root_children[i + 1];

        if key == "ImageSetting" {
            // ImageSetting is a dict containing ImageCorrection
            let setting_children = element_children(val);
            let mut j = 0;
            while j + 1 < setting_children.len() {
                let skey = get_element_text(setting_children[j]).unwrap_or_default();
                if skey == "ImageCorrection" {
                    return Some(parse_image_correction(setting_children[j + 1]));
                }
                j += 2;
            }
        }
        i += 2;
    }

    None
}
