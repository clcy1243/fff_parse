use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

/// Byte order of the TIFF file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    LittleEndian,
    BigEndian,
}

/// TIFF data types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TiffType {
    Byte = 1,
    Ascii = 2,
    Short = 3,
    Long = 4,
    Rational = 5,
    SByte = 6,
    Undefined = 7,
    SShort = 8,
    SLong = 9,
    SRational = 10,
    Float = 11,
    Double = 12,
}

impl TiffType {
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::Byte),
            2 => Some(Self::Ascii),
            3 => Some(Self::Short),
            4 => Some(Self::Long),
            5 => Some(Self::Rational),
            6 => Some(Self::SByte),
            7 => Some(Self::Undefined),
            8 => Some(Self::SShort),
            9 => Some(Self::SLong),
            10 => Some(Self::SRational),
            11 => Some(Self::Float),
            12 => Some(Self::Double),
            _ => None,
        }
    }

    pub fn size(self) -> usize {
        match self {
            Self::Byte | Self::Ascii | Self::SByte | Self::Undefined => 1,
            Self::Short | Self::SShort => 2,
            Self::Long | Self::SLong | Self::Float => 4,
            Self::Rational | Self::SRational | Self::Double => 8,
        }
    }
}

/// Parsed tag value
#[derive(Debug, Clone)]
pub enum TagValue {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SByte(Vec<i8>),
    Undefined(Vec<u8>),
    SShort(Vec<i16>),
    SLong(Vec<i32>),
    SRational(Vec<(i32, i32)>),
    Float(Vec<f32>),
    Double(Vec<f64>),
}

impl TagValue {
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            TagValue::Byte(v) => v.first().map(|&x| x as u32),
            TagValue::Short(v) => v.first().map(|&x| x as u32),
            TagValue::Long(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_u32_vec(&self) -> Vec<u32> {
        match self {
            TagValue::Byte(v) => v.iter().map(|&x| x as u32).collect(),
            TagValue::Short(v) => v.iter().map(|&x| x as u32).collect(),
            TagValue::Long(v) => v.clone(),
            _ => vec![],
        }
    }

    pub fn as_string(&self) -> Option<String> {
        match self {
            TagValue::Ascii(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn as_rational(&self) -> Option<(u32, u32)> {
        match self {
            TagValue::Rational(v) => v.first().copied(),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_srational(&self) -> Option<(i32, i32)> {
        match self {
            TagValue::SRational(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TagValue::Byte(v) => v.first().map(|&x| x as f64),
            TagValue::Short(v) => v.first().map(|&x| x as f64),
            TagValue::Long(v) => v.first().map(|&x| x as f64),
            TagValue::Rational(v) => v.first().map(|(n, d)| {
                if *d == 0 {
                    0.0
                } else {
                    *n as f64 / *d as f64
                }
            }),
            TagValue::SRational(v) => v.first().map(|(n, d)| {
                if *d == 0 {
                    0.0
                } else {
                    *n as f64 / *d as f64
                }
            }),
            TagValue::Float(v) => v.first().map(|&x| x as f64),
            TagValue::Double(v) => v.first().copied(),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            TagValue::Byte(v) | TagValue::Undefined(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for TagValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagValue::Ascii(s) => write!(f, "{}", s),
            TagValue::Byte(v) if v.len() == 1 => write!(f, "{}", v[0]),
            TagValue::Short(v) if v.len() == 1 => write!(f, "{}", v[0]),
            TagValue::Long(v) if v.len() == 1 => write!(f, "{}", v[0]),
            TagValue::Rational(v) if v.len() == 1 => {
                let (n, d) = v[0];
                if d == 0 {
                    write!(f, "0")
                } else if n % d == 0 {
                    write!(f, "{}", n / d)
                } else {
                    write!(f, "{}/{}", n, d)
                }
            }
            TagValue::SRational(v) if v.len() == 1 => {
                let (n, d) = v[0];
                if d == 0 {
                    write!(f, "0")
                } else {
                    write!(f, "{}/{}", n, d)
                }
            }
            TagValue::Float(v) if v.len() == 1 => write!(f, "{:.4}", v[0]),
            TagValue::Double(v) if v.len() == 1 => write!(f, "{:.4}", v[0]),
            TagValue::Byte(v) => write!(f, "[{} bytes]", v.len()),
            TagValue::Short(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| x.to_string()).collect();
                if v.len() > 8 {
                    write!(f, "[{}, ... ({} total)]", items.join(", "), v.len())
                } else {
                    write!(f, "[{}]", items.join(", "))
                }
            }
            TagValue::Long(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| x.to_string()).collect();
                if v.len() > 8 {
                    write!(f, "[{}, ... ({} total)]", items.join(", "), v.len())
                } else {
                    write!(f, "[{}]", items.join(", "))
                }
            }
            TagValue::Rational(v) => {
                let items: Vec<String> = v
                    .iter()
                    .take(8)
                    .map(|(n, d)| format!("{}/{}", n, d))
                    .collect();
                if v.len() > 8 {
                    write!(f, "[{}, ... ({} total)]", items.join(", "), v.len())
                } else {
                    write!(f, "[{}]", items.join(", "))
                }
            }
            TagValue::SRational(v) => {
                let items: Vec<String> = v
                    .iter()
                    .take(8)
                    .map(|(n, d)| format!("{}/{}", n, d))
                    .collect();
                if v.len() > 8 {
                    write!(f, "[{}, ... ({} total)]", items.join(", "), v.len())
                } else {
                    write!(f, "[{}]", items.join(", "))
                }
            }
            TagValue::SByte(v) => write!(f, "[{} sbytes]", v.len()),
            TagValue::Undefined(v) => write!(f, "[{} bytes]", v.len()),
            TagValue::SShort(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| x.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            TagValue::SLong(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| x.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            TagValue::Float(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| format!("{:.4}", x)).collect();
                write!(f, "[{}]", items.join(", "))
            }
            TagValue::Double(v) => {
                let items: Vec<String> = v.iter().take(8).map(|x| format!("{:.4}", x)).collect();
                write!(f, "[{}]", items.join(", "))
            }
        }
    }
}

/// A single IFD entry
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IfdEntry {
    pub tag: u16,
    pub tiff_type: u16,
    pub count: u32,
    pub value: TagValue,
}

/// An Image File Directory
#[derive(Debug, Clone)]
pub struct Ifd {
    pub name: String,
    pub entries: BTreeMap<u16, IfdEntry>,
}

impl Ifd {
    pub fn get(&self, tag: u16) -> Option<&TagValue> {
        self.entries.get(&tag).map(|e| &e.value)
    }

    pub fn get_u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(|v| v.as_u32())
    }

    pub fn get_string(&self, tag: u16) -> Option<String> {
        self.get(tag).and_then(|v| v.as_string())
    }
}

/// Top-level parsed TIFF/FFF file
#[derive(Debug, Clone)]
pub struct TiffFile {
    pub byte_order: ByteOrder,
    pub magic: u16,
    pub ifds: Vec<Ifd>,
    /// Extracted preview JPEG bytes (if found)
    pub preview_jpeg: Option<Vec<u8>>,
    /// The raw file data (for extracting image regions)
    data: Vec<u8>,
}

/// Binary reader with configurable byte order
struct TiffReader<R: Read + Seek> {
    reader: R,
    byte_order: ByteOrder,
}

impl<R: Read + Seek> TiffReader<R> {
    fn new(reader: R, byte_order: ByteOrder) -> Self {
        Self { reader, byte_order }
    }

    #[allow(dead_code)]
    fn read_u8(&mut self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        self.reader.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_u16(&mut self) -> io::Result<u16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u16::from_le_bytes(buf),
            ByteOrder::BigEndian => u16::from_be_bytes(buf),
        })
    }

    fn read_u32(&mut self) -> io::Result<u32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u32::from_le_bytes(buf),
            ByteOrder::BigEndian => u32::from_be_bytes(buf),
        })
    }

    fn read_i16(&mut self) -> io::Result<i16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i16::from_le_bytes(buf),
            ByteOrder::BigEndian => i16::from_be_bytes(buf),
        })
    }

    fn read_i32(&mut self) -> io::Result<i32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i32::from_le_bytes(buf),
            ByteOrder::BigEndian => i32::from_be_bytes(buf),
        })
    }

    fn read_f32(&mut self) -> io::Result<f32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => f32::from_le_bytes(buf),
            ByteOrder::BigEndian => f32::from_be_bytes(buf),
        })
    }

    fn read_f64(&mut self) -> io::Result<f64> {
        let mut buf = [0u8; 8];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => f64::from_le_bytes(buf),
            ByteOrder::BigEndian => f64::from_be_bytes(buf),
        })
    }

    fn read_bytes(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; count];
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn seek(&mut self, pos: u64) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Start(pos))
    }

    fn position(&mut self) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Current(0))
    }

    fn read_value(
        &mut self,
        tiff_type: TiffType,
        count: u32,
        value_offset: u32,
    ) -> io::Result<TagValue> {
        let total_size = tiff_type.size() * count as usize;
        let inline = total_size <= 4;

        if !inline {
            self.seek(value_offset as u64)?;
        }

        let count = count as usize;

        match tiff_type {
            TiffType::Byte => {
                let data = self.read_bytes(count)?;
                Ok(TagValue::Byte(data))
            }
            TiffType::Ascii => {
                let data = self.read_bytes(count)?;
                let s = String::from_utf8_lossy(&data)
                    .trim_end_matches('\0')
                    .to_string();
                Ok(TagValue::Ascii(s))
            }
            TiffType::Short => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_u16()?);
                }
                Ok(TagValue::Short(vals))
            }
            TiffType::Long => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_u32()?);
                }
                Ok(TagValue::Long(vals))
            }
            TiffType::Rational => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    let n = self.read_u32()?;
                    let d = self.read_u32()?;
                    vals.push((n, d));
                }
                Ok(TagValue::Rational(vals))
            }
            TiffType::SByte => {
                let data = self.read_bytes(count)?;
                Ok(TagValue::SByte(data.into_iter().map(|b| b as i8).collect()))
            }
            TiffType::Undefined => {
                let data = self.read_bytes(count)?;
                Ok(TagValue::Undefined(data))
            }
            TiffType::SShort => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_i16()?);
                }
                Ok(TagValue::SShort(vals))
            }
            TiffType::SLong => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_i32()?);
                }
                Ok(TagValue::SLong(vals))
            }
            TiffType::SRational => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    let n = self.read_i32()?;
                    let d = self.read_i32()?;
                    vals.push((n, d));
                }
                Ok(TagValue::SRational(vals))
            }
            TiffType::Float => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_f32()?);
                }
                Ok(TagValue::Float(vals))
            }
            TiffType::Double => {
                let mut vals = Vec::with_capacity(count);
                for _ in 0..count {
                    vals.push(self.read_f64()?);
                }
                Ok(TagValue::Double(vals))
            }
        }
    }
}

impl TiffFile {
    /// Open and parse a TIFF/FFF file (read-only).
    /// The file is read entirely into memory; the original file is never modified.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let data = fs::read(path)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> io::Result<Self> {
        if data.len() < 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small"));
        }

        // Detect byte order
        let byte_order = match (data[0], data[1]) {
            (0x49, 0x49) => ByteOrder::LittleEndian, // "II" — Intel
            (0x4D, 0x4D) => ByteOrder::BigEndian,    // "MM" — Motorola
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid byte order mark: {:02X} {:02X}", data[0], data[1]),
                ));
            }
        };

        let mut reader = TiffReader::new(Cursor::new(data), byte_order);
        reader.seek(2)?;

        // Read magic number — standard TIFF is 42 (0x2A), Imacon FFF uses 0x55 ('U')
        let magic = reader.read_u16()?;
        if magic != 0x002A && magic != 0x0055 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Unsupported TIFF magic: 0x{:04X} (expected 0x002A or 0x0055)",
                    magic
                ),
            ));
        }

        let first_ifd_offset = reader.read_u32()?;

        let mut ifds = Vec::new();
        let mut ifd_offset = first_ifd_offset;
        let mut ifd_index = 0;

        while ifd_offset != 0 && (ifd_offset as usize) < data.len() {
            let (ifd, next_offset) =
                Self::parse_ifd(&mut reader, ifd_offset, &format!("IFD#{}", ifd_index))?;

            // Parse sub-IFDs if present (tag 0x014A)
            let sub_ifd_offsets: Vec<u32> = ifd
                .get(0x014A)
                .map(|v| v.as_u32_vec())
                .unwrap_or_default();

            // Parse EXIF IFD if present (tag 0x8769)
            let exif_ifd_offset = ifd.get_u32(0x8769);

            ifds.push(ifd);

            for (si, &sub_offset) in sub_ifd_offsets.iter().enumerate() {
                if sub_offset != 0 && (sub_offset as usize) < data.len() {
                    if let Ok((sub_ifd, _)) = Self::parse_ifd(
                        &mut reader,
                        sub_offset,
                        &format!("IFD#{}:Sub#{}", ifd_index, si),
                    ) {
                        ifds.push(sub_ifd);
                    }
                }
            }

            if let Some(exif_offset) = exif_ifd_offset {
                if exif_offset != 0 && (exif_offset as usize) < data.len() {
                    if let Ok((exif_ifd, _)) =
                        Self::parse_ifd(&mut reader, exif_offset, "EXIF")
                    {
                        // Parse MakerNote sub-IFD if present
                        let makernote_entry = exif_ifd.get(0x927C).cloned();
                        ifds.push(exif_ifd);

                        if let Some(TagValue::Undefined(mn_data)) = makernote_entry {
                            if let Ok(mn_ifd) =
                                Self::parse_makernote(&mn_data, byte_order, data)
                            {
                                ifds.push(mn_ifd);
                            }
                        }
                    }
                }
            }

            ifd_offset = next_offset;
            ifd_index += 1;

            if ifd_index > 20 {
                break; // safety limit
            }
        }

        // Extract preview JPEG
        let preview_jpeg = Self::extract_preview(data, &ifds);

        Ok(TiffFile {
            byte_order,
            magic,
            ifds,
            preview_jpeg,
            data: data.to_vec(),
        })
    }

    fn parse_ifd(
        reader: &mut TiffReader<Cursor<&[u8]>>,
        offset: u32,
        name: &str,
    ) -> io::Result<(Ifd, u32)> {
        reader.seek(offset as u64)?;
        let entry_count = reader.read_u16()?;

        if entry_count > 1000 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Too many IFD entries: {}", entry_count),
            ));
        }

        let mut entries = BTreeMap::new();

        for _ in 0..entry_count {
            let tag = reader.read_u16()?;
            let tiff_type_raw = reader.read_u16()?;
            let count = reader.read_u32()?;

            let value_offset_pos = reader.position()?;
            let value_offset = reader.read_u32()?;
            let next_entry_pos = reader.position()?;

            if let Some(tiff_type) = TiffType::from_u16(tiff_type_raw) {
                let total_size = tiff_type.size() as u64 * count as u64;
                let inline = total_size <= 4;

                // For inline values, seek back to read from the offset field itself
                if inline {
                    reader.seek(value_offset_pos)?;
                }

                match reader.read_value(tiff_type, count, value_offset) {
                    Ok(value) => {
                        entries.insert(
                            tag,
                            IfdEntry {
                                tag,
                                tiff_type: tiff_type_raw,
                                count,
                                value,
                            },
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to read tag 0x{:04X}: {}", tag, e);
                    }
                }
            }

            reader.seek(next_entry_pos)?;
        }

        let next_ifd_offset = reader.read_u32().unwrap_or(0);

        Ok((
            Ifd {
                name: name.to_string(),
                entries,
            },
            next_ifd_offset,
        ))
    }

    fn parse_makernote(mn_data: &[u8], byte_order: ByteOrder, _file_data: &[u8]) -> io::Result<Ifd> {
        // Hasselblad MakerNote is typically a standard TIFF IFD structure
        // Sometimes it has an "HASSELBLAD\0" header prefix
        let mut offset = 0usize;

        // Check for known headers
        if mn_data.len() > 12 {
            // Check for "HASSELBLAD\0" or similar prefix
            if let Ok(header) = std::str::from_utf8(&mn_data[..10.min(mn_data.len())]) {
                if header.starts_with("HASSELBLAD") || header.starts_with("Hasselblad") {
                    // Skip the header — find the IFD start
                    offset = mn_data
                        .iter()
                        .position(|&b| b == 0)
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    // Align to even
                    if offset % 2 != 0 {
                        offset += 1;
                    }
                }
            }
        }

        if offset + 2 > mn_data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MakerNote too short",
            ));
        }

        let mut reader = TiffReader::new(Cursor::new(&mn_data[offset..]), byte_order);
        let entry_count = reader.read_u16()?;

        if entry_count > 500 || entry_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid MakerNote entry count: {}", entry_count),
            ));
        }

        let mut entries = BTreeMap::new();

        for _ in 0..entry_count {
            if reader.position()? + 12 > mn_data.len() as u64 - offset as u64 {
                break;
            }

            let tag = reader.read_u16()?;
            let tiff_type_raw = reader.read_u16()?;
            let count = reader.read_u32()?;

            let value_offset_pos = reader.position()?;
            let value_offset = reader.read_u32()?;
            let next_entry_pos = reader.position()?;

            if let Some(tiff_type) = TiffType::from_u16(tiff_type_raw) {
                let total_size = tiff_type.size() as u64 * count as u64;

                // Safety: skip huge values
                if total_size > 65536 {
                    reader.seek(next_entry_pos)?;
                    continue;
                }

                let inline = total_size <= 4;

                if inline {
                    reader.seek(value_offset_pos)?;
                }

                match reader.read_value(tiff_type, count, value_offset.wrapping_sub(offset as u32))
                {
                    Ok(value) => {
                        entries.insert(
                            tag,
                            IfdEntry {
                                tag,
                                tiff_type: tiff_type_raw,
                                count,
                                value,
                            },
                        );
                    }
                    Err(_) => {
                        // MakerNote offsets can be relative to different bases — try file-relative
                        if !inline {
                            reader.seek(value_offset_pos)?;
                            // Just skip this value
                        }
                    }
                }
            }

            reader.seek(next_entry_pos)?;
        }

        Ok(Ifd {
            name: "MakerNote".to_string(),
            entries,
        })
    }

    fn extract_preview(data: &[u8], ifds: &[Ifd]) -> Option<Vec<u8>> {
        // Strategy 1: Look for JPEGInterchangeFormat in any IFD
        for ifd in ifds {
            if let (Some(jpeg_offset), Some(jpeg_len)) =
                (ifd.get_u32(0x0201), ifd.get_u32(0x0202))
            {
                let offset = jpeg_offset as usize;
                let len = jpeg_len as usize;
                if offset + len <= data.len() && len > 0 {
                    let jpeg_data = &data[offset..offset + len];
                    // Verify JPEG signature
                    if jpeg_data.len() >= 2 && jpeg_data[0] == 0xFF && jpeg_data[1] == 0xD8 {
                        return Some(jpeg_data.to_vec());
                    }
                }
            }
        }

        // Strategy 2: Use StripOffsets + StripByteCounts from preview IFD
        // The first IFD in FFF files is often the preview (RGB, compressed)
        for ifd in ifds {
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            // JPEG compression (6 or 7)
            if compression == 6 || compression == 7 {
                if let Some(strip_offsets) = ifd.get(0x0111) {
                    let offsets = strip_offsets.as_u32_vec();
                    if let Some(strip_counts) = ifd.get(0x0117) {
                        let counts = strip_counts.as_u32_vec();
                        if !offsets.is_empty() && !counts.is_empty() {
                            let offset = offsets[0] as usize;
                            let count: usize = counts.iter().map(|&c| c as usize).sum();
                            if offset + count <= data.len() {
                                let jpeg_data = &data[offset..offset + count];
                                if jpeg_data.len() >= 2
                                    && jpeg_data[0] == 0xFF
                                    && jpeg_data[1] == 0xD8
                                {
                                    return Some(jpeg_data.to_vec());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Strategy 3: Scan for embedded JPEG by looking for JFIF/Exif markers
        Self::find_embedded_jpeg(data)
    }

    fn find_embedded_jpeg(data: &[u8]) -> Option<Vec<u8>> {
        // Look for JPEG SOI marker (0xFF 0xD8) followed by APP0/APP1
        let mut pos = 0;
        let mut best: Option<(usize, usize)> = None;

        while pos + 4 < data.len() {
            if data[pos] == 0xFF && data[pos + 1] == 0xD8 {
                // Possible JPEG start
                if data[pos + 2] == 0xFF
                    && (data[pos + 3] == 0xE0
                        || data[pos + 3] == 0xE1
                        || data[pos + 3] == 0xDB)
                {
                    // Find JPEG end (EOI: 0xFF 0xD9)
                    if let Some(eoi_pos) = Self::find_jpeg_end(data, pos) {
                        let size = eoi_pos - pos + 2;
                        // Take the largest JPEG as the preview
                        if best.map_or(true, |(_, bs)| size > bs) {
                            best = Some((pos, size));
                        }
                    }
                }
            }
            pos += 1;
        }

        best.map(|(offset, size)| data[offset..offset + size].to_vec())
    }

    fn find_jpeg_end(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start + 2;
        while pos + 1 < data.len() {
            if data[pos] == 0xFF && data[pos + 1] == 0xD9 {
                return Some(pos);
            }
            pos += 1;
        }
        None
    }

    #[allow(dead_code)]
    pub fn file_size(&self) -> usize {
        self.data.len()
    }

    /// Access the raw file data (for external parsers like FlexColor history)
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }

    /// Get a summary of all metadata as key-value pairs
    pub fn metadata_summary(&self) -> Vec<(String, String)> {
        use crate::tags::*;

        let mut result = Vec::new();

        // File info
        let format_type = if self.magic == 0x0055 {
            "Imacon FFF (3F)"
        } else {
            "TIFF-based FFF"
        };
        result.push(("Format".into(), format_type.into()));
        result.push((
            "Byte Order".into(),
            match self.byte_order {
                ByteOrder::LittleEndian => "Little-Endian (Intel)",
                ByteOrder::BigEndian => "Big-Endian (Motorola)",
            }
            .into(),
        ));
        result.push(("File Size".into(), format_file_size(self.data.len())));
        result.push(("IFD Count".into(), self.ifds.len().to_string()));

        // Key metadata from IFDs
        if let Some(v) = self.find_tag_string(0x010F) {
            result.push(("Make".into(), v));
        }
        if let Some(v) = self.find_tag_string(0x0110) {
            result.push(("Model".into(), v));
        }
        if let Some(v) = self.find_tag_string(0x0131) {
            result.push(("Software".into(), v));
        }
        if let Some(v) = self.find_tag_string(0x0132) {
            result.push(("DateTime".into(), v));
        }
        if let Some(v) = self.find_tag_string(0x9003) {
            result.push(("DateTimeOriginal".into(), v));
        }

        // Image dimensions (from main raw IFD, usually the largest sub-IFD)
        let (width, height) = self.best_dimensions();
        if width > 0 && height > 0 {
            result.push(("Dimensions".into(), format!("{} × {}", width, height)));
        }

        // Thumbnail dimensions (from reduced-resolution IFD)
        for ifd in &self.ifds {
            let subfile_type = ifd.get_u32(0x00FE).unwrap_or(0);
            if subfile_type == 1 {
                let tw = ifd.get_u32(0x0100).unwrap_or(0);
                let th = ifd.get_u32(0x0101).unwrap_or(0);
                if tw > 0 && th > 0 {
                    result.push(("Thumbnail".into(), format!("{} × {}", tw, th)));
                }
                break;
            }
        }

        // Bits per sample
        if let Some(bps) = self.find_tag_value(0x0102) {
            result.push(("BitsPerSample".into(), bps.to_string()));
        }

        // Compression
        if let Some(v) = self.find_tag_u32(0x0103) {
            result.push(("Compression".into(), compression_name(v).into()));
        }

        // Photometric interpretation
        if let Some(v) = self.find_tag_u32(0x0106) {
            result.push(("Photometric".into(), photometric_name(v).into()));
        }

        // Orientation
        if let Some(v) = self.find_tag_u32(0x0112) {
            result.push(("Orientation".into(), orientation_name(v).into()));
        }

        // Resolution
        if let Some(v) = self.find_tag_value(0x011A) {
            if let Some(res) = v.as_f64() {
                let unit = self.find_tag_u32(0x0128).unwrap_or(2);
                let unit_str = match unit {
                    1 => "",
                    2 => " dpi",
                    3 => " dpcm",
                    _ => "",
                };
                result.push(("XResolution".into(), format!("{:.0}{}", res, unit_str)));
            }
        }
        if let Some(v) = self.find_tag_value(0x011B) {
            if let Some(res) = v.as_f64() {
                let unit = self.find_tag_u32(0x0128).unwrap_or(2);
                let unit_str = match unit {
                    1 => "",
                    2 => " dpi",
                    3 => " dpcm",
                    _ => "",
                };
                result.push(("YResolution".into(), format!("{:.0}{}", res, unit_str)));
            }
        }

        // EXIF data
        if let Some(v) = self.find_tag_value(0x829A) {
            if let Some((n, d)) = v.as_rational() {
                if d != 0 {
                    if n == 1 {
                        result.push(("ExposureTime".into(), format!("1/{}", d)));
                    } else {
                        result.push((
                            "ExposureTime".into(),
                            format!("{:.4}s", n as f64 / d as f64),
                        ));
                    }
                }
            }
        }
        if let Some(v) = self.find_tag_value(0x829D) {
            if let Some(fnum) = v.as_f64() {
                result.push(("FNumber".into(), format!("f/{:.1}", fnum)));
            }
        }
        if let Some(v) = self.find_tag_u32(0x8827) {
            result.push(("ISO".into(), v.to_string()));
        }
        if let Some(v) = self.find_tag_value(0x920A) {
            if let Some(fl) = v.as_f64() {
                result.push(("FocalLength".into(), format!("{:.1} mm", fl)));
            }
        }
        if let Some(v) = self.find_tag_u32(0x9207) {
            result.push(("MeteringMode".into(), metering_mode_name(v).into()));
        }

        // Serial numbers
        if let Some(v) = self.find_tag_string(0xA431) {
            result.push(("BodySerialNumber".into(), v));
        }
        if let Some(v) = self.find_tag_string(0xC62F) {
            result.push(("CameraSerialNumber".into(), v));
        }

        // Hasselblad MakerNote data
        for ifd in &self.ifds {
            if ifd.name == "MakerNote" {
                if let Some(v) = ifd.get_u32(0x0005) {
                    result.push(("WhiteBalance".into(), white_balance_name(v).into()));
                }
                if let Some(v) = ifd.get_string(0x0028) {
                    result.push(("PhocusVersion".into(), v));
                }
            }
        }

        result
    }

    /// Get all IFD entries as displayable list (for detail view)
    pub fn all_tags(&self) -> Vec<(String, String, String, String)> {
        use crate::tags::*;

        let mut result = Vec::new();

        for ifd in &self.ifds {
            for entry in ifd.entries.values() {
                let name = if ifd.name == "MakerNote" {
                    makernote_tag_name(entry.tag)
                        .unwrap_or_else(|| standard_tag_name(entry.tag).unwrap_or("Unknown"))
                } else {
                    standard_tag_name(entry.tag).unwrap_or("Unknown")
                };

                result.push((
                    ifd.name.clone(),
                    format!("0x{:04X}", entry.tag),
                    name.to_string(),
                    entry.value.to_string(),
                ));
            }
        }

        result
    }

    fn find_tag_value(&self, tag: u16) -> Option<&TagValue> {
        for ifd in &self.ifds {
            if let Some(v) = ifd.get(tag) {
                return Some(v);
            }
        }
        None
    }

    fn find_tag_u32(&self, tag: u16) -> Option<u32> {
        self.find_tag_value(tag).and_then(|v| v.as_u32())
    }

    fn find_tag_string(&self, tag: u16) -> Option<String> {
        self.find_tag_value(tag).and_then(|v| v.as_string())
    }

    fn best_dimensions(&self) -> (u32, u32) {
        let mut best_w = 0u32;
        let mut best_h = 0u32;
        for ifd in &self.ifds {
            let w = ifd.get_u32(0x0100).unwrap_or(0);
            let h = ifd.get_u32(0x0101).unwrap_or(0);
            if (w as u64) * (h as u64) > (best_w as u64) * (best_h as u64) {
                best_w = w;
                best_h = h;
            }
        }
        (best_w, best_h)
    }

    /// Try to decode the raw image data as an image::DynamicImage
    /// Falls back to preview JPEG if raw decoding fails
    pub fn decode_preview_image(&self) -> Option<image::DynamicImage> {
        // First try: use preview JPEG
        if let Some(jpeg_data) = &self.preview_jpeg {
            if let Ok(img) =
                image::load_from_memory_with_format(jpeg_data, image::ImageFormat::Jpeg)
            {
                return Some(img);
            }
        }

        // Collect candidate IFDs: prefer thumbnail (NewSubfileType=1), then small, then large
        let mut candidates: Vec<(usize, u64, bool)> = Vec::new(); // (ifd_index, pixel_count, is_thumbnail)
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let width = ifd.get_u32(0x0100).unwrap_or(0) as u64;
            let height = ifd.get_u32(0x0101).unwrap_or(0) as u64;
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            let photometric = ifd.get_u32(0x0106).unwrap_or(0);
            let spp = ifd.get_u32(0x0115).unwrap_or(1);
            let subfile_type = ifd.get_u32(0x00FE).unwrap_or(0);

            if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
                let is_thumb = subfile_type == 1;
                candidates.push((idx, width * height, is_thumb));
            }
        }

        // Sort: prefer larger images for better quality preview
        // All candidates are uncompressed RGB and decodable
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        for (idx, _pixels, _is_thumb) in &candidates {
            if let Some(img) = self.decode_uncompressed_rgb(&self.ifds[*idx]) {
                return Some(img);
            }
        }

        // Last resort: try to read as TIFF using the image crate
        if let Ok(img) = image::load_from_memory_with_format(&self.data, image::ImageFormat::Tiff) {
            return Some(img);
        }

        None
    }

    /// Decode the largest preview image, subsampled to fit within `max_dim` pixels.
    /// Reads directly from file data without intermediate full-resolution buffers.
    /// Returns a 16-bit or 8-bit image at reduced resolution for fast display.
    pub fn decode_preview_downscaled(&self, max_dim: u32) -> Option<image::DynamicImage> {
        // First try: use preview JPEG (already small)
        if let Some(jpeg_data) = &self.preview_jpeg {
            if let Ok(img) =
                image::load_from_memory_with_format(jpeg_data, image::ImageFormat::Jpeg)
            {
                return Some(img);
            }
        }

        // Find the largest uncompressed RGB IFD
        let mut best: Option<(usize, u64)> = None;
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let width = ifd.get_u32(0x0100).unwrap_or(0) as u64;
            let height = ifd.get_u32(0x0101).unwrap_or(0) as u64;
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            let photometric = ifd.get_u32(0x0106).unwrap_or(0);
            let spp = ifd.get_u32(0x0115).unwrap_or(1);

            if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
                let pixels = width * height;
                if best.is_none() || pixels > best.unwrap().1 {
                    best = Some((idx, pixels));
                }
            }
        }

        let (idx, _) = best?;
        self.decode_ifd_downscaled(&self.ifds[idx], max_dim)
    }

    /// Decode an IFD's uncompressed RGB data, subsampled by a nearest-neighbor factor
    /// to fit within `max_dim`. Reads directly from `self.data` to avoid copying
    /// the full strip data into an intermediate buffer.
    fn decode_ifd_downscaled(&self, ifd: &Ifd, max_dim: u32) -> Option<image::DynamicImage> {
        let width = ifd.get_u32(0x0100)? as usize;
        let height = ifd.get_u32(0x0101)? as usize;
        let bps = ifd.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(8);

        let strip_offsets = ifd.get(0x0111)?;
        let offsets = strip_offsets.as_u32_vec();
        let strip_counts = ifd.get(0x0117)?;
        let counts = strip_counts.as_u32_vec();

        if offsets.is_empty() || counts.is_empty() {
            return None;
        }

        let src_pixel_bytes: usize = if bps == 16 { 6 } else { 3 }; // bytes per pixel (3 channels)
        let src_row_bytes = width * src_pixel_bytes;

        // Calculate subsample factor
        let factor = if width as u32 > max_dim || height as u32 > max_dim {
            let fw = (width as f64 / max_dim as f64).ceil() as usize;
            let fh = (height as f64 / max_dim as f64).ceil() as usize;
            fw.max(fh).max(1)
        } else {
            1
        };

        let out_w = (width + factor - 1) / factor;
        let out_h = (height + factor - 1) / factor;

        // Build row-to-file-offset lookup (maps each source row to its byte offset in self.data)
        let mut row_file_offsets: Vec<usize> = Vec::with_capacity(height);
        for (off, cnt) in offsets.iter().zip(counts.iter()) {
            let rows_in_strip = (*cnt as usize) / src_row_bytes;
            for r in 0..rows_in_strip {
                row_file_offsets.push(*off as usize + r * src_row_bytes);
            }
            if row_file_offsets.len() >= height {
                break;
            }
        }
        if row_file_offsets.len() < height {
            row_file_offsets.resize(height, 0);
        }

        log::info!(
            "decode_ifd_downscaled: {}x{} → {}x{} (factor={})",
            width, height, out_w, out_h, factor
        );

        if bps == 16 {
            use rayon::prelude::*;
            let byte_order = self.byte_order;
            let out_row_len = out_w * 3; // u16 values per output row
            let mut rgb16 = vec![0u16; out_row_len * out_h];
            let data = &self.data;

            rgb16
                .par_chunks_mut(out_row_len)
                .enumerate()
                .for_each(|(out_y, row)| {
                    let src_y = out_y * factor;
                    if src_y >= height {
                        return;
                    }
                    let row_start = row_file_offsets[src_y];
                    for out_x in 0..out_w {
                        let src_x = out_x * factor;
                        if src_x >= width {
                            break;
                        }
                        let base_idx = row_start + src_x * 6; // 3 channels × 2 bytes
                        for ch in 0..3 {
                            let byte_idx = base_idx + ch * 2;
                            if byte_idx + 1 < data.len() {
                                row[out_x * 3 + ch] = match byte_order {
                                    ByteOrder::LittleEndian => u16::from_le_bytes([
                                        data[byte_idx],
                                        data[byte_idx + 1],
                                    ]),
                                    ByteOrder::BigEndian => u16::from_be_bytes([
                                        data[byte_idx],
                                        data[byte_idx + 1],
                                    ]),
                                };
                            }
                        }
                    }
                });

            let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                out_w as u32,
                out_h as u32,
                rgb16,
            )?;
            return Some(image::DynamicImage::ImageRgb16(img));
        } else if bps == 8 {
            let out_row_len = out_w * 3;
            let mut rgb8 = vec![0u8; out_row_len * out_h];
            let data = &self.data;

            for out_y in 0..out_h {
                let src_y = out_y * factor;
                if src_y >= height {
                    break;
                }
                let row_start = row_file_offsets[src_y];
                for out_x in 0..out_w {
                    let src_x = out_x * factor;
                    if src_x >= width {
                        break;
                    }
                    let base_idx = row_start + src_x * 3;
                    for ch in 0..3 {
                        let idx = base_idx + ch;
                        if idx < data.len() {
                            rgb8[out_y * out_row_len + out_x * 3 + ch] = data[idx];
                        }
                    }
                }
            }

            let img = image::RgbImage::from_raw(out_w as u32, out_h as u32, rgb8)?;
            return Some(image::DynamicImage::ImageRgb8(img));
        }

        None
    }

    /// Decode the smallest available thumbnail for grid/filmstrip display.
    /// Prefers IFD with NewSubfileType=1 (thumbnail), then the smallest IFD.
    /// Decode the FlexColor pre-rendered 8-bit thumbnail (NewSubfileType=1).
    /// This thumbnail has full FlexColor processing baked in (ICC, saturation,
    /// curves, levels) and is the authoritative "correct" look.
    pub fn decode_thumbnail(&self) -> Option<image::DynamicImage> {
        if let Some(jpeg_data) = &self.preview_jpeg {
            if let Ok(img) =
                image::load_from_memory_with_format(jpeg_data, image::ImageFormat::Jpeg)
            {
                return Some(img);
            }
        }

        let mut candidates: Vec<(usize, u64, bool)> = Vec::new();
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let width = ifd.get_u32(0x0100).unwrap_or(0) as u64;
            let height = ifd.get_u32(0x0101).unwrap_or(0) as u64;
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            let photometric = ifd.get_u32(0x0106).unwrap_or(0);
            let spp = ifd.get_u32(0x0115).unwrap_or(1);
            let subfile_type = ifd.get_u32(0x00FE).unwrap_or(0);

            if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
                let is_thumb = subfile_type == 1;
                candidates.push((idx, width * height, is_thumb));
            }
        }

        // Sort: prefer thumbnails (NewSubfileType=1) first, then smallest image
        candidates.sort_by(|a, b| {
            b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1))
        });

        for (idx, _pixels, _is_thumb) in &candidates {
            if let Some(img) = self.decode_uncompressed_rgb(&self.ifds[*idx]) {
                return Some(img);
            }
        }

        None
    }

    fn decode_uncompressed_rgb(&self, ifd: &Ifd) -> Option<image::DynamicImage> {
        let width = ifd.get_u32(0x0100)? as u32;
        let height = ifd.get_u32(0x0101)? as u32;
        let bps = ifd.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(8);

        let strip_offsets = ifd.get(0x0111)?;
        let offsets = strip_offsets.as_u32_vec();
        let strip_counts = ifd.get(0x0117)?;
        let counts = strip_counts.as_u32_vec();

        if offsets.is_empty() || counts.is_empty() {
            return None;
        }

        let mut pixel_data = Vec::new();
        for (off, cnt) in offsets.iter().zip(counts.iter()) {
            let start = *off as usize;
            let end = start + *cnt as usize;
            if end <= self.data.len() {
                pixel_data.extend_from_slice(&self.data[start..end]);
            }
        }

        if bps == 8 {
            let expected = (width as usize) * (height as usize) * 3;
            if pixel_data.len() >= expected {
                let img = image::RgbImage::from_raw(width, height, pixel_data[..expected].to_vec())?;
                return Some(image::DynamicImage::ImageRgb8(img));
            }
        } else if bps == 16 {
            // Return native 16-bit data, parallelised row-by-row
            use rayon::prelude::*;
            let pixel_count = (width as usize) * (height as usize) * 3;
            let byte_count = pixel_count * 2;
            if pixel_data.len() < byte_count {
                return None;
            }
            let byte_order = self.byte_order;
            let row_pixels = width as usize * 3; // u16 values per row
            let row_bytes = row_pixels * 2;
            let mut rgb16 = vec![0u16; pixel_count];

            rgb16
                .par_chunks_mut(row_pixels)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_bytes;
                    for i in 0..row_pixels {
                        let idx = src_start + i * 2;
                        row[i] = match byte_order {
                            ByteOrder::LittleEndian => {
                                u16::from_le_bytes([pixel_data[idx], pixel_data[idx + 1]])
                            }
                            ByteOrder::BigEndian => {
                                u16::from_be_bytes([pixel_data[idx], pixel_data[idx + 1]])
                            }
                        };
                    }
                });

            let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                width, height, rgb16,
            )?;
            return Some(image::DynamicImage::ImageRgb16(img));
        }

        None
    }

    /// Decode the full-resolution image for TIFF export.
    /// Preserves 16-bit data as Rgb16 when possible (no downscale to 8-bit).
    pub fn decode_for_export(&self) -> Option<image::DynamicImage> {
        // Find the largest uncompressed RGB IFD (the main raw image)
        let mut best: Option<(usize, u64)> = None;
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let width = ifd.get_u32(0x0100).unwrap_or(0) as u64;
            let height = ifd.get_u32(0x0101).unwrap_or(0) as u64;
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            let photometric = ifd.get_u32(0x0106).unwrap_or(0);
            let spp = ifd.get_u32(0x0115).unwrap_or(1);

            if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
                let pixels = width * height;
                if best.is_none() || pixels > best.unwrap().1 {
                    best = Some((idx, pixels));
                }
            }
        }

        let (idx, _) = best?;
        let ifd = &self.ifds[idx];
        let width = ifd.get_u32(0x0100)? as u32;
        let height = ifd.get_u32(0x0101)? as u32;
        let bps = ifd.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(8);

        let strip_offsets = ifd.get(0x0111)?;
        let offsets = strip_offsets.as_u32_vec();
        let strip_counts = ifd.get(0x0117)?;
        let counts = strip_counts.as_u32_vec();

        if offsets.is_empty() || counts.is_empty() {
            return None;
        }

        let mut pixel_data = Vec::new();
        for (off, cnt) in offsets.iter().zip(counts.iter()) {
            let start = *off as usize;
            let end = start + *cnt as usize;
            if end <= self.data.len() {
                pixel_data.extend_from_slice(&self.data[start..end]);
            }
        }

        if bps == 16 {
            let pixel_count = (width as usize) * (height as usize) * 3;
            let mut rgb16 = Vec::with_capacity(pixel_count);
            let byte_order = self.byte_order;
            for i in 0..pixel_count {
                let idx = i * 2;
                if idx + 1 < pixel_data.len() {
                    let val = match byte_order {
                        ByteOrder::LittleEndian => {
                            u16::from_le_bytes([pixel_data[idx], pixel_data[idx + 1]])
                        }
                        ByteOrder::BigEndian => {
                            u16::from_be_bytes([pixel_data[idx], pixel_data[idx + 1]])
                        }
                    };
                    rgb16.push(val);
                }
            }
            if rgb16.len() >= pixel_count {
                let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                    width, height, rgb16,
                )?;
                return Some(image::DynamicImage::ImageRgb16(img));
            }
        } else if bps == 8 {
            let expected = (width as usize) * (height as usize) * 3;
            if pixel_data.len() >= expected {
                let img = image::RgbImage::from_raw(width, height, pixel_data[..expected].to_vec())?;
                return Some(image::DynamicImage::ImageRgb8(img));
            }
        }

        None
    }
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
