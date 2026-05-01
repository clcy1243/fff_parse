//! TIFF/FFF 文件解析器。
//!
//! 解析 Imacon/Hasselblad Flextight X5 扫描仪产出的 TIFF 和 FFF 文件。
//! 支持标准 TIFF（magic 0x2A）和 Imacon FFF（magic 0x55）两种格式，
//! 可读取 IFD 链、EXIF、MakerNote，并提取预览图像。
//!
//! # FFF 文件二进制布局
//!
//! FFF 文件基于 TIFF 格式，每个文件固定包含 3 个 IFD（与编辑历史条目数无关）。
//! 编辑历史仅以 XML 元数据形式存储，不会重复保存图像像素。
//!
//! ```text
//! 偏移             内容                                  大小（典型值）
//! ─────────────────────────────────────────────────────────────────────
//! [0..8]           TIFF Header（字节序 MM + magic 0x55   8 B
//!                  + IFD#0 偏移指针）
//! [8..~75]         tag 0xB4C7: FlexColor 版本/序列号      ~67 B
//! [~76..~400076]   tag 0xC519: XML Plist 容器              400 KB（预分配）
//!   ├ 前 4 字节     XML 实际长度（大端 u32）
//!   ├ XML plist     编辑历史（ImageSettings 数组）          ~30 KB
//!   └ 零填充        预留扩展空间                           ~370 KB
//! [~400076..~600076] tag 0xB4C5: 二进制编辑设置副本          ~195 KB
//! [~600076..]      IFD#0 tag 表 + 全分辨率 16-bit RGB      文件主体（~92 MB/帧）
//! [...]            IFD#1 tag 表 + 8-bit 缩略图              ~1.3 MB
//!                  （FlexColor 预渲染，含完整处理效果）
//! [...]            tag 0xC51A: CCD 校准数据                 ~211 KB
//! [...]            IFD#2 tag 表 + 16-bit 降采样预览          ~2.6 MB
//! [末尾]           IPTC / 其他尾部元数据                    ~200 B
//! ```
//!
//! ## 自定义 Tag 说明
//!
//! | Tag      | 名称           | 内容说明                                      |
//! |----------|----------------|-----------------------------------------------|
//! | 0xB4C7   | FlexColor 版本 | ASCII: "English v. X.Y.Z (PC)" + 扫描仪序列号 |
//! | 0xC519   | XML 设置容器   | 前 4 字节为实际 XML 长度，后跟 plist XML       |
//! | 0xB4C5   | 二进制设置     | 编辑设置的二进制序列化格式                      |
//! | 0xC51A   | CCD 校准       | 扫描仪 CCD 传感器校准/配置数据                 |
//!
//! ## IFD 结构
//!
//! | IFD  | SubfileType | 位深  | 用途                                          |
//! |------|-------------|-------|-----------------------------------------------|
//! | #0   | 0 (主图)    | 16-bit | 全分辨率原始扫描数据（最大，占文件 95%+）     |
//! | #1   | 1 (缩略图)  | 8-bit  | FlexColor 预渲染缩略图（含色彩处理效果）       |
//! | #2   | 0 (主图)    | 16-bit | 降采样预览（与 IFD#0 同比例缩小）             |
//!
//! 文件大小差异主要取决于扫描尺寸：整卷扫描 (3996×15118) ≈ 347 MB，
//! 单帧 (3601×4489) ≈ 97 MB。

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

/// TIFF 文件的字节序
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// 小端序
    LittleEndian,
    /// 大端序
    BigEndian,
}

/// TIFF 数据类型
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
    /// 将 u16 值转换为对应的 TiffType，无效值返回 None
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

    /// 返回该类型单个值占用的字节数
    pub fn size(self) -> usize {
        match self {
            Self::Byte | Self::Ascii | Self::SByte | Self::Undefined => 1,
            Self::Short | Self::SShort => 2,
            Self::Long | Self::SLong | Self::Float => 4,
            Self::Rational | Self::SRational | Self::Double => 8,
        }
    }
}

/// 已解析的 TIFF 标签值，每种变体对应一种 TIFF 数据类型
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
    /// 取第一个值并转换为 u32，不适用的类型返回 None
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            TagValue::Byte(v) => v.first().map(|&x| x as u32),
            TagValue::Short(v) => v.first().map(|&x| x as u32),
            TagValue::Long(v) => v.first().copied(),
            _ => None,
        }
    }

    /// 将所有值转换为 u32 向量，不适用的类型返回空向量
    pub fn as_u32_vec(&self) -> Vec<u32> {
        match self {
            TagValue::Byte(v) => v.iter().map(|&x| x as u32).collect(),
            TagValue::Short(v) => v.iter().map(|&x| x as u32).collect(),
            TagValue::Long(v) => v.clone(),
            _ => vec![],
        }
    }

    /// 转换为字符串，仅 Ascii 类型有效
    pub fn as_string(&self) -> Option<String> {
        match self {
            TagValue::Ascii(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// 取第一个无符号有理数值 (分子, 分母)
    pub fn as_rational(&self) -> Option<(u32, u32)> {
        match self {
            TagValue::Rational(v) => v.first().copied(),
            _ => None,
        }
    }

    /// 取第一个有符号有理数值 (分子, 分母)
    #[allow(dead_code)]
    pub fn as_srational(&self) -> Option<(i32, i32)> {
        match self {
            TagValue::SRational(v) => v.first().copied(),
            _ => None,
        }
    }

    /// 将第一个值转换为 f64，支持整型、有理数和浮点类型
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

    /// 获取原始字节切片，仅 Byte 和 Undefined 类型有效
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

/// 单个 IFD 条目，包含标签号、类型、计数和解析后的值
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IfdEntry {
    /// 标签编号
    pub tag: u16,
    /// TIFF 数据类型编号
    pub tiff_type: u16,
    /// 值的数量
    pub count: u32,
    /// 已解析的标签值
    pub value: TagValue,
}

/// 图像文件目录 (IFD)，包含目录名称和所有条目
#[derive(Debug, Clone)]
pub struct Ifd {
    /// 目录名称（如 "IFD#0"、"EXIF"、"MakerNote"）
    pub name: String,
    /// 按标签号排序的条目映射
    pub entries: BTreeMap<u16, IfdEntry>,
}

impl Ifd {
    /// 根据标签号获取标签值的引用
    pub fn get(&self, tag: u16) -> Option<&TagValue> {
        self.entries.get(&tag).map(|e| &e.value)
    }

    /// 根据标签号获取值并转换为 u32
    pub fn get_u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(|v| v.as_u32())
    }

    /// 根据标签号获取值并转换为字符串
    pub fn get_string(&self, tag: u16) -> Option<String> {
        self.get(tag).and_then(|v| v.as_string())
    }
}

/// 顶层 TIFF/FFF 文件结构，包含字节序、IFD 列表和预览图像
#[derive(Debug, Clone)]
pub struct TiffFile {
    /// 文件字节序
    pub byte_order: ByteOrder,
    /// 魔数（0x2A 为标准 TIFF，0x55 为 Imacon FFF）
    pub magic: u16,
    /// 所有解析出的 IFD（含子 IFD、EXIF、MakerNote）
    pub ifds: Vec<Ifd>,
    /// 提取的预览 JPEG 数据（如存在）
    pub preview_jpeg: Option<Vec<u8>>,
    /// 原始文件数据（用于提取图像区域）
    data: Vec<u8>,
}

/// 支持可配置字节序的二进制读取器
struct TiffReader<R: Read + Seek> {
    reader: R,
    byte_order: ByteOrder,
}

impl<R: Read + Seek> TiffReader<R> {
    /// 创建指定字节序的读取器
    fn new(reader: R, byte_order: ByteOrder) -> Self {
        Self { reader, byte_order }
    }

    /// 读取一个 u8 值
    #[allow(dead_code)]
    fn read_u8(&mut self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        self.reader.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    /// 按当前字节序读取一个 u16 值
    fn read_u16(&mut self) -> io::Result<u16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u16::from_le_bytes(buf),
            ByteOrder::BigEndian => u16::from_be_bytes(buf),
        })
    }

    /// 按当前字节序读取一个 u32 值
    fn read_u32(&mut self) -> io::Result<u32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => u32::from_le_bytes(buf),
            ByteOrder::BigEndian => u32::from_be_bytes(buf),
        })
    }

    /// 按当前字节序读取一个 i16 值
    fn read_i16(&mut self) -> io::Result<i16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i16::from_le_bytes(buf),
            ByteOrder::BigEndian => i16::from_be_bytes(buf),
        })
    }

    /// 按当前字节序读取一个 i32 值
    fn read_i32(&mut self) -> io::Result<i32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => i32::from_le_bytes(buf),
            ByteOrder::BigEndian => i32::from_be_bytes(buf),
        })
    }

    /// 按当前字节序读取一个 f32 值
    fn read_f32(&mut self) -> io::Result<f32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => f32::from_le_bytes(buf),
            ByteOrder::BigEndian => f32::from_be_bytes(buf),
        })
    }

    /// 按当前字节序读取一个 f64 值
    fn read_f64(&mut self) -> io::Result<f64> {
        let mut buf = [0u8; 8];
        self.reader.read_exact(&mut buf)?;
        Ok(match self.byte_order {
            ByteOrder::LittleEndian => f64::from_le_bytes(buf),
            ByteOrder::BigEndian => f64::from_be_bytes(buf),
        })
    }

    /// 读取指定长度的字节数组
    fn read_bytes(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; count];
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// 跳转到指定的绝对位置
    fn seek(&mut self, pos: u64) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Start(pos))
    }

    /// 获取当前读取位置
    fn position(&mut self) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Current(0))
    }

    /// 根据 TIFF 类型和数量读取标签值，内联值（≤4字节）直接从当前位置读取，
    /// 否则跳转到 value_offset 指定的偏移量处读取
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
    /// 打开并解析 TIFF/FFF 文件（只读），文件内容全部读入内存。
    ///
    /// 适用于需要访问全分辨率像素数据的场景（详情查看、导出）。
    /// 对于仅需缩略图的场景，请使用 [`open_for_thumbnail`] 节省内存。
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let data = fs::read(path)?;
        Self::parse(&data)
    }

    /// 轻量级打开 FFF 文件并仅解码缩略图，不将完整文件加载到内存。
    ///
    /// 通过文件 seeking 只读取 IFD 元数据和缩略图像素区域，
    /// 内存占用约为完整加载的 1/50～1/100（例如 97 MB 文件仅需读 ~1.3 MB）。
    /// 优先选择 FlexColor 预渲染的 8-bit 缩略图（IFD#1, SubfileType=1）。
    pub fn open_for_thumbnail<P: AsRef<Path>>(path: P) -> io::Result<Option<image::DynamicImage>> {
        use std::io::BufReader;

        let file = fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        // 读取 TIFF 头：字节序 + magic + 首个 IFD 偏移
        let mut header = [0u8; 8];
        reader.read_exact(&mut header)?;

        let byte_order = match (header[0], header[1]) {
            (0x49, 0x49) => ByteOrder::LittleEndian,
            (0x4D, 0x4D) => ByteOrder::BigEndian,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid byte order")),
        };

        let read_u16_bo = |buf: &[u8], off: usize| -> u16 {
            match byte_order {
                ByteOrder::LittleEndian => u16::from_le_bytes([buf[off], buf[off + 1]]),
                ByteOrder::BigEndian => u16::from_be_bytes([buf[off], buf[off + 1]]),
            }
        };
        let read_u32_bo = |buf: &[u8], off: usize| -> u32 {
            match byte_order {
                ByteOrder::LittleEndian => u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]),
                ByteOrder::BigEndian => u32::from_be_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]),
            }
        };

        let magic = read_u16_bo(&header, 2);
        if magic != 0x002A && magic != 0x0055 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a TIFF/FFF file"));
        }

        let first_ifd = read_u32_bo(&header, 4);

        /// 轻量级 IFD 信息，仅包含缩略图选择和解码所需的字段
        struct LightIfd {
            width: u32,
            height: u32,
            bps: u32,
            compression: u32,
            photometric: u32,
            spp: u32,
            subfile_type: u32,
            strip_offsets: Vec<u32>,
            strip_byte_counts: Vec<u32>,
        }

        let mut light_ifds = Vec::new();
        let mut ifd_offset = first_ifd;

        // 遍历 IFD 链，仅解析缩略图选择所需的 tag
        for _ in 0..20 {
            if ifd_offset == 0 || ifd_offset as u64 >= file_len {
                break;
            }

            reader.seek(SeekFrom::Start(ifd_offset as u64))?;
            let mut buf2 = [0u8; 2];
            reader.read_exact(&mut buf2)?;
            let entry_count = read_u16_bo(&buf2, 0) as usize;
            if entry_count > 1000 {
                break;
            }

            // 一次性读取所有 IFD 条目（每条 12 字节）+ 下一 IFD 偏移（4 字节）
            let entries_size = entry_count * 12;
            let mut entries_buf = vec![0u8; entries_size + 4];
            reader.read_exact(&mut entries_buf)?;

            let mut lifd = LightIfd {
                width: 0, height: 0, bps: 8, compression: 1,
                photometric: 0, spp: 1, subfile_type: 0,
                strip_offsets: Vec::new(), strip_byte_counts: Vec::new(),
            };

            // 需要额外 seek 读取的外部值: (文件偏移, 条目数)
            let mut strip_offsets_ext: Option<(u32, u32)> = None;
            let mut strip_counts_ext: Option<(u32, u32)> = None;
            let mut bps_ext: Option<u32> = None;

            for i in 0..entry_count {
                let e = i * 12;
                let tag = read_u16_bo(&entries_buf, e);
                let typ = read_u16_bo(&entries_buf, e + 2);
                let count = read_u32_bo(&entries_buf, e + 4);
                let value_offset = read_u32_bo(&entries_buf, e + 8);
                let short_val = read_u16_bo(&entries_buf, e + 8);

                match tag {
                    0x00FE => lifd.subfile_type = value_offset,
                    0x0100 => lifd.width = if typ == 3 { short_val as u32 } else { value_offset },
                    0x0101 => lifd.height = if typ == 3 { short_val as u32 } else { value_offset },
                    0x0102 => {
                        if count == 1 {
                            lifd.bps = short_val as u32;
                        } else {
                            bps_ext = Some(value_offset);
                        }
                    }
                    0x0103 => lifd.compression = if typ == 3 { short_val as u32 } else { value_offset },
                    0x0106 => lifd.photometric = if typ == 3 { short_val as u32 } else { value_offset },
                    0x0111 => {
                        if count == 1 {
                            lifd.strip_offsets = vec![value_offset];
                        } else {
                            strip_offsets_ext = Some((value_offset, count));
                        }
                    }
                    0x0115 => lifd.spp = if typ == 3 { short_val as u32 } else { value_offset },
                    0x0117 => {
                        if count == 1 {
                            lifd.strip_byte_counts = vec![value_offset];
                        } else {
                            strip_counts_ext = Some((value_offset, count));
                        }
                    }
                    _ => {} // 跳过所有非必需 tag（包括大型 0xC519 等）
                }
            }

            // 读取外部 tag 值（小量 seek + 小量读取）
            if let Some(offset) = bps_ext {
                reader.seek(SeekFrom::Start(offset as u64))?;
                let mut buf = [0u8; 2];
                reader.read_exact(&mut buf)?;
                lifd.bps = read_u16_bo(&buf, 0) as u32;
            }
            if let Some((offset, count)) = strip_offsets_ext {
                let count = (count as usize).min(65536);
                reader.seek(SeekFrom::Start(offset as u64))?;
                let mut buf = vec![0u8; count * 4];
                reader.read_exact(&mut buf)?;
                lifd.strip_offsets = (0..count).map(|i| read_u32_bo(&buf, i * 4)).collect();
            }
            if let Some((offset, count)) = strip_counts_ext {
                let count = (count as usize).min(65536);
                reader.seek(SeekFrom::Start(offset as u64))?;
                let mut buf = vec![0u8; count * 4];
                reader.read_exact(&mut buf)?;
                lifd.strip_byte_counts = (0..count).map(|i| read_u32_bo(&buf, i * 4)).collect();
            }

            // 下一 IFD 偏移
            ifd_offset = read_u32_bo(&entries_buf, entries_size);
            light_ifds.push(lifd);
        }

        // 选择最佳缩略图：SubfileType=1 + 8-bit 优先，然后最小尺寸
        let mut best: Option<(usize, u64, bool, bool)> = None;
        for (idx, ifd) in light_ifds.iter().enumerate() {
            if ifd.compression != 1 || ifd.photometric != 2 || ifd.spp < 3
                || ifd.width == 0 || ifd.height == 0 || ifd.strip_offsets.is_empty()
            {
                continue;
            }
            // Skip IFDs too large for thumbnail use (GPU max texture side is typically 16384)
            if ifd.width > 4096 || ifd.height > 4096 {
                continue;
            }
            let pixels = ifd.width as u64 * ifd.height as u64;
            let is_thumb = ifd.subfile_type == 1;
            let is_8bit = ifd.bps == 8;

            let better = if let Some((_, bp, bt, b8)) = best {
                if is_thumb && !bt { true }
                else if is_thumb == bt && is_8bit && !b8 { true }
                else if is_thumb == bt && is_8bit == b8 && pixels < bp { true }
                else { false }
            } else {
                true
            };

            if better {
                best = Some((idx, pixels, is_thumb, is_8bit));
            }
        }

        let (idx, _, _, _) = match best {
            Some(b) => b,
            None => return Ok(None),
        };

        let ifd = &light_ifds[idx];

        // 仅读取缩略图的像素数据（通常 ~1.3 MB）
        let total_bytes: usize = ifd.strip_byte_counts.iter().map(|c| *c as usize).sum();
        let mut pixel_data = Vec::with_capacity(total_bytes);
        for (off, cnt) in ifd.strip_offsets.iter().zip(ifd.strip_byte_counts.iter()) {
            reader.seek(SeekFrom::Start(*off as u64))?;
            let mut buf = vec![0u8; *cnt as usize];
            reader.read_exact(&mut buf)?;
            pixel_data.extend_from_slice(&buf);
        }

        let width = ifd.width;
        let height = ifd.height;

        if ifd.bps == 8 {
            let expected = width as usize * height as usize * 3;
            if pixel_data.len() >= expected {
                pixel_data.truncate(expected);
                if let Some(img) = image::RgbImage::from_raw(width, height, pixel_data) {
                    return Ok(Some(image::DynamicImage::ImageRgb8(img)));
                }
            }
        } else if ifd.bps == 16 {
            let expected_u16 = width as usize * height as usize * 3;
            let pixels_u16: Vec<u16> = pixel_data.chunks_exact(2)
                .take(expected_u16)
                .map(|c| match byte_order {
                    ByteOrder::LittleEndian => u16::from_le_bytes([c[0], c[1]]),
                    ByteOrder::BigEndian => u16::from_be_bytes([c[0], c[1]]),
                })
                .collect();
            if pixels_u16.len() >= expected_u16 {
                if let Some(img) = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                    width, height, pixels_u16,
                ) {
                    return Ok(Some(image::DynamicImage::ImageRgb16(img)));
                }
            }
        }

        Ok(None)
    }

    /// 从字节切片解析 TIFF/FFF 数据，检测字节序和魔数后遍历 IFD 链
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

    /// 解析指定偏移量处的单个 IFD，返回 (IFD, 下一个IFD偏移量)
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

    /// 解析 Hasselblad MakerNote 子 IFD，处理可能存在的 "HASSELBLAD" 头部前缀
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

    /// 从文件数据中提取预览 JPEG，依次尝试 JPEGInterchangeFormat、
    /// JPEG 压缩的 Strip 数据、以及扫描嵌入的 JPEG 标记
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

        // Strategy 3: byte-by-byte scan（已禁用 —— 对 97MB FFF 需 ~100ms 且
        // 匹配到的 JPEG 多是非预览数据，decode 失败后仍走 RGB IFD 路径）。
        // FFF_SCAN_EMBEDDED_JPEG=1 环境变量可临时启用（调试非标准 FFF 用）。
        if std::env::var("FFF_SCAN_EMBEDDED_JPEG").is_ok() {
            return Self::find_embedded_jpeg(data);
        }
        None
    }

    /// 扫描字节数据查找嵌入的 JPEG（SOI + APP0/APP1/DQT），返回最大的一个
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

    /// 从指定起始位置查找 JPEG EOI 标记 (0xFF 0xD9) 的位置
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

    /// 获取文件大小（字节数）
    #[allow(dead_code)]
    pub fn file_size(&self) -> usize {
        self.data.len()
    }

    /// 获取原始文件数据的引用（供外部解析器使用）
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }

    /// 释放原始文件数据以回收内存。
    ///
    /// 在预览解码、元数据提取等操作完成后调用，释放占用大量内存的像素缓冲区。
    /// 调用后 `raw_data()` 返回空切片，`decode_*` 方法将无法工作。
    pub fn release_data(&mut self) {
        self.data = Vec::new();
        self.preview_jpeg = None;
    }

    /// 从 tag 0xC519 中提取 XML plist 字符串。
    ///
    /// tag 0xC519 的前 4 字节为 XML 实际长度（大端 u32），后跟 XML 数据。
    /// 利用已解析的 IFD tag 精确定位 XML，避免全文件扫描。
    pub fn settings_xml(&self) -> Option<String> {
        // 在 IFD#0 中查找 tag 0xC519
        let tag_value = self.ifds.first()?.get(0xC519)?;
        if let TagValue::Byte(ref raw) | TagValue::Undefined(ref raw) = tag_value {
            if raw.len() < 4 {
                return None;
            }
            // 前 4 字节为 XML 实际长度（大端）
            let xml_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
            let xml_len = xml_len.min(raw.len() - 4);
            if xml_len == 0 {
                return None;
            }
            let xml_bytes = &raw[4..4 + xml_len];
            std::str::from_utf8(xml_bytes).ok().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// 汇总所有元数据为键值对列表（格式、字节序、尺寸、EXIF 等）
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

    /// 获取所有 IFD 条目用于详情展示，返回 (IFD名, 标签hex, 标签名, 值) 列表
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

    /// 在所有 IFD 中查找指定标签的值
    fn find_tag_value(&self, tag: u16) -> Option<&TagValue> {
        for ifd in &self.ifds {
            if let Some(v) = ifd.get(tag) {
                return Some(v);
            }
        }
        None
    }

    /// 在所有 IFD 中查找指定标签并转换为 u32
    fn find_tag_u32(&self, tag: u16) -> Option<u32> {
        self.find_tag_value(tag).and_then(|v| v.as_u32())
    }

    /// 在所有 IFD 中查找指定标签并转换为字符串
    fn find_tag_string(&self, tag: u16) -> Option<String> {
        self.find_tag_value(tag).and_then(|v| v.as_string())
    }

    /// 查找所有 IFD 中像素数最多的图像尺寸
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

    /// 解码预览图像为 DynamicImage，优先使用预览 JPEG，
    /// 其次尝试未压缩 RGB 数据，最后尝试 image crate 直接解码
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

    /// 解码最大的预览图像并降采样到 `max_dim` 像素内，
    /// 直接从文件数据读取，避免中间全分辨率缓冲区
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

    /// 解码指定 IFD 的未压缩 RGB 数据，通过最近邻降采样到 `max_dim` 像素内，
    /// 直接从 `self.data` 读取以避免拷贝完整 Strip 数据
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

    /// 解码用于网格/胶片条显示的缩略图。
    /// 优先使用预览 JPEG，其次选择 FlexColor 预渲染的 8 位缩略图
    /// （NewSubfileType=1），该缩略图包含完整的 FlexColor 处理效果。
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

    /// 解码指定 IFD 的未压缩 RGB 数据为 DynamicImage，支持 8 位和 16 位
    pub fn decode_uncompressed_rgb(&self, ifd: &Ifd) -> Option<image::DynamicImage> {
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

    /// 同时解码 8-bit 缩略图（IFD#1, SubfileType=1）和同分辨率的 16-bit 预览（IFD#2）。
    /// 用于负片胶片曲线提取：8-bit 缩略图包含 FlexColor 完整处理效果，
    /// 16-bit 预览为原始扫描数据（降采样），两者分辨率一致可逐像素比对。
    pub fn decode_thumbnail_pair(
        &self,
    ) -> Option<(
        image::RgbImage,
        image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    )> {
        // 收集所有未压缩 RGB IFD 的信息
        let mut ifds_info: Vec<(usize, u32, u32, u32, u32)> = Vec::new(); // (idx, w, h, bps, subfile_type)
        for (idx, ifd) in self.ifds.iter().enumerate() {
            let width = ifd.get_u32(0x0100).unwrap_or(0);
            let height = ifd.get_u32(0x0101).unwrap_or(0);
            let bps = ifd.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(8);
            let compression = ifd.get_u32(0x0103).unwrap_or(1);
            let photometric = ifd.get_u32(0x0106).unwrap_or(0);
            let spp = ifd.get_u32(0x0115).unwrap_or(1);
            let subfile_type = ifd.get_u32(0x00FE).unwrap_or(0);

            if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
                ifds_info.push((idx, width, height, bps, subfile_type));
            }
        }

        // 找到 8-bit 缩略图 (SubfileType=1)
        let thumb_info = ifds_info.iter().find(|i| i.4 == 1 && i.3 == 8)?;
        let thumb_w = thumb_info.1;
        let thumb_h = thumb_info.2;

        // 找到同分辨率的 16-bit 预览 (SubfileType=0, 非最大尺寸)
        let max_pixels = ifds_info
            .iter()
            .map(|i| i.1 as u64 * i.2 as u64)
            .max()
            .unwrap_or(0);
        let preview_info = ifds_info.iter().find(|i| {
            i.3 == 16
                && i.4 == 0
                && (i.1 as u64 * i.2 as u64) < max_pixels
                && i.1 == thumb_w
                && i.2 == thumb_h
        })?;

        // 解码两个 IFD
        let thumb_img = self.decode_uncompressed_rgb(&self.ifds[thumb_info.0])?;
        let preview_img = self.decode_uncompressed_rgb(&self.ifds[preview_info.0])?;

        let thumb_8 = thumb_img.to_rgb8();
        let preview_16 = match preview_img {
            image::DynamicImage::ImageRgb16(rgb16) => rgb16,
            _ => return None,
        };

        if thumb_8.width() != preview_16.width() || thumb_8.height() != preview_16.height() {
            return None;
        }

        Some((thumb_8, preview_16))
    }

    /// 解码全分辨率图像用于 TIFF 导出，保留 16 位数据不降级为 8 位
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

/// 将字节数格式化为人类可读的文件大小字符串（B/KB/MB/GB）
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
