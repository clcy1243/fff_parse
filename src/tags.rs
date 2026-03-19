//! TIFF 标签 ID 到可读名称的映射，包含标准 TIFF 标签和哈苏 MakerNote 标签。

use std::collections::HashMap;

/// 根据标准 TIFF 标签 ID 查找对应的可读名称
pub fn standard_tag_name(tag: u16) -> Option<&'static str> {
    STANDARD_TAGS.get(&tag).copied()
}

/// 根据哈苏 MakerNote 标签 ID 查找对应的可读名称
pub fn makernote_tag_name(tag: u16) -> Option<&'static str> {
    MAKERNOTE_TAGS.get(&tag).copied()
}

/// 将方向值转换为可读名称
pub fn orientation_name(v: u32) -> &'static str {
    match v {
        1 => "Normal",
        2 => "Flip Horizontal",
        3 => "Rotate 180",
        4 => "Flip Vertical",
        5 => "Transpose",
        6 => "Rotate 90 CW",
        7 => "Transverse",
        8 => "Rotate 270 CW",
        _ => "Unknown",
    }
}

/// 将压缩类型值转换为可读名称
pub fn compression_name(v: u32) -> &'static str {
    match v {
        1 => "Uncompressed",
        2 => "CCITT 1D",
        3 => "Group 3 Fax",
        4 => "Group 4 Fax",
        5 => "LZW",
        6 => "JPEG (old)",
        7 => "JPEG",
        8 => "Deflate",
        32773 => "PackBits",
        34713 => "Nikon NEF",
        65535 => "Hasselblad Lossless",
        _ => "Unknown",
    }
}

/// 将光度解释值转换为可读名称
pub fn photometric_name(v: u32) -> &'static str {
    match v {
        0 => "WhiteIsZero",
        1 => "BlackIsZero",
        2 => "RGB",
        3 => "Palette",
        4 => "Transparency Mask",
        5 => "CMYK",
        6 => "YCbCr",
        8 => "CIELab",
        32803 => "CFA (Color Filter Array)",
        34892 => "LinearRaw",
        _ => "Unknown",
    }
}

/// 将测光模式值转换为可读名称
pub fn metering_mode_name(v: u32) -> &'static str {
    match v {
        0 => "Unknown",
        1 => "Average",
        2 => "Center Weighted",
        3 => "Spot",
        4 => "Multi-Spot",
        5 => "Multi-Segment",
        6 => "Partial",
        255 => "Other",
        _ => "Unknown",
    }
}

/// 将白平衡值转换为可读名称
pub fn white_balance_name(v: u32) -> &'static str {
    match v {
        1 => "Auto",
        2 => "Daylight",
        3 => "Tungsten",
        4 => "Fluorescent",
        5 => "Flash",
        6 => "Manual",
        10 => "Cloudy",
        11 => "Shade",
        _ => "Unknown",
    }
}

use std::sync::LazyLock;

/// 标准 TIFF 标签 ID 到名称的映射表
static STANDARD_TAGS: LazyLock<HashMap<u16, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        // Basic TIFF tags
        (0x00FE, "NewSubfileType"),
        (0x00FF, "SubfileType"),
        (0x0100, "ImageWidth"),
        (0x0101, "ImageLength"),
        (0x0102, "BitsPerSample"),
        (0x0103, "Compression"),
        (0x0106, "PhotometricInterpretation"),
        (0x010E, "ImageDescription"),
        (0x010F, "Make"),
        (0x0110, "Model"),
        (0x0111, "StripOffsets"),
        (0x0112, "Orientation"),
        (0x0115, "SamplesPerPixel"),
        (0x0116, "RowsPerStrip"),
        (0x0117, "StripByteCounts"),
        (0x011A, "XResolution"),
        (0x011B, "YResolution"),
        (0x011C, "PlanarConfiguration"),
        (0x0128, "ResolutionUnit"),
        (0x0131, "Software"),
        (0x0132, "DateTime"),
        (0x013B, "Artist"),
        (0x013D, "Predictor"),
        (0x014A, "SubIFDs"),
        (0x0153, "SampleFormat"),
        // JPEG tags
        (0x0201, "JPEGInterchangeFormat"),
        (0x0202, "JPEGInterchangeFormatLength"),
        // EXIF tags
        (0x8298, "Copyright"),
        (0x829A, "ExposureTime"),
        (0x829D, "FNumber"),
        (0x8769, "ExifIFD"),
        (0x8822, "ExposureProgram"),
        (0x8827, "ISOSpeedRatings"),
        (0x9000, "ExifVersion"),
        (0x9003, "DateTimeOriginal"),
        (0x9004, "DateTimeDigitized"),
        (0x9201, "ShutterSpeedValue"),
        (0x9202, "ApertureValue"),
        (0x9204, "ExposureBiasValue"),
        (0x9205, "MaxApertureValue"),
        (0x9207, "MeteringMode"),
        (0x9209, "Flash"),
        (0x920A, "FocalLength"),
        (0x927C, "MakerNote"),
        (0xA001, "ColorSpace"),
        (0xA002, "PixelXDimension"),
        (0xA003, "PixelYDimension"),
        (0xA420, "ImageUniqueID"),
        (0xA431, "BodySerialNumber"),
        (0xA432, "LensInfo"),
        (0xA434, "LensModel"),
        (0xA435, "LensSerialNumber"),
        // DNG / CFA tags
        (0xC612, "DNGVersion"),
        (0xC613, "DNGBackwardVersion"),
        (0xC614, "UniqueCameraModel"),
        (0xC621, "ColorMatrix1"),
        (0xC622, "ColorMatrix2"),
        (0xC623, "CameraCalibration1"),
        (0xC624, "CameraCalibration2"),
        (0xC628, "AsShotNeutral"),
        (0xC62F, "CameraSerialNumber"),
        (0xC65A, "CalibrationIlluminant1"),
        (0xC65B, "CalibrationIlluminant2"),
        (0xC65D, "RawDataUniqueID"),
        // ICC Profile
        (0x8773, "ICCProfile"),
        // IPTC / Photoshop / Imacon tags
        (0x83BB, "IPTC-NAA"),
        (0x8568, "IPTC-NAA (alt)"),
        (0x8649, "PhotoshopImageResources"),
        (0xB4C5, "ImaconRawData"),
        (0xB4C7, "ImaconScanInfo"),
        (0xC519, "ImaconCalibration"),
        (0xC51A, "ImaconProfileData"),
    ])
});

/// 哈苏 MakerNote 标签 ID 到名称的映射表
static MAKERNOTE_TAGS: LazyLock<HashMap<u16, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        (0x0005, "WhiteBalance"),
        (0x0013, "Quality"),
        (0x0015, "Model"),
        (0x0017, "CameraInfo"),
        (0x0018, "LensInfo"),
        (0x0028, "PhocusVersion"),
        (0x002A, "ColorMatrix"),
        (0x0046, "Gain"),
        (0x0047, "FocusPoint"),
        (0x004A, "ShutterType"),
        (0x0059, "CropMode"),
        (0x005B, "DriveMode"),
        (0x005C, "ReleaseCount"),
        (0x0061, "LensSerialNumber"),
        (0x0063, "ExactExposureTime"),
    ])
});
