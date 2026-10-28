pub mod encoder;
pub mod decoder;

pub const MAGIC_WRITE: &[u8; 4] = b"PSPK";
pub const MAGIC_LEGACY: &[u8; 4] = b"PYGR";
#[allow(dead_code)]
pub const MAGIC: &[u8; 4] = MAGIC_WRITE;
pub const FORMAT_VERSION: u16 = 1;
pub const SCHEMA_VERSION: u32 = 0;
// FLAG_HMAC: payload ends with a 32-byte HMAC-SHA256 tag (see integrity.rs).
// FLAG_COMPRESSED: reserved, never set.
#[allow(dead_code)]
pub const FLAG_HMAC: u8 = 1 << 0;
#[allow(dead_code)]
pub const FLAG_COMPRESSED: u8 = 1 << 1;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    None = 0x01,
    True = 0x02,
    False = 0x03,
    Int = 0x04,
    Float = 0x05,
    String = 0x06,
    Bytes = 0x07,
    List = 0x08,
    Tuple = 0x09,
    Dict = 0x0A,
    Set = 0x0B,
    FrozenSet = 0x0C,
    Dataclass = 0x0D,
    Reference = 0x0E,
    BigInt = 0x0F,
    ByteArray = 0x10,
    Complex = 0x11,
    StrRaw = 0x12,
    // ---- stdlib value types (0x13..). All integers little-endian; "id" is a
    // u32 record id; "blob" is a u32 length followed by that many bytes. ----
    /// datetime.datetime: year u16, month u8, day u8, hour u8, minute u8,
    /// second u8, microsecond u32, fold u8, tzinfo id (u32::MAX = naive;
    /// otherwise must be a Timezone or ZoneInfo record). 16 bytes.
    DateTime = 0x13,
    /// datetime.date: year u16, month u8, day u8.
    Date = 0x14,
    /// datetime.time: hour u8, minute u8, second u8, microsecond u32, fold u8,
    /// tzinfo id (as for DateTime). 12 bytes.
    Time = 0x15,
    /// datetime.timedelta: days i32, seconds u32, microseconds u32.
    TimeDelta = 0x16,
    /// datetime.timezone: offset as TimeDelta payload (12 bytes), has_name u8,
    /// then (if has_name) the name as a UTF-8 (surrogatepass) blob.
    Timezone = 0x17,
    /// zoneinfo.ZoneInfo: key as UTF-8 blob; rebuilt with ZoneInfo(key), keys
    /// that are absolute or contain '', '..' / '.' components or NUL are rejected.
    ZoneInfo = 0x18,
    /// decimal.Decimal: str(d) as ASCII blob (covers NaN/sNaN/Infinity/-0).
    Decimal = 0x19,
    /// uuid.UUID: 16 raw bytes (UUID.bytes).
    Uuid = 0x1A,
    /// fractions.Fraction: numerator id, denominator id (both int records).
    Fraction = 0x1B,
    /// range: start id, stop id, step id (all int records).
    Range = 0x1C,
    /// slice: start id, stop id, step id (any records).
    Slice = 0x1D,
    /// The Ellipsis singleton. No payload.
    Ellipsis = 0x1E,
    /// The NotImplemented singleton. No payload.
    NotImplemented = 0x1F,
    /// pathlib path: flavor u8 (0 PurePosixPath, 1 PureWindowsPath,
    /// 2 PosixPath, 3 WindowsPath), str(path) as UTF-8 (surrogatepass) blob.
    /// Concrete flavors load as the concrete class when it matches the
    /// running OS, otherwise as the corresponding Pure* class.
    Path = 0x20,
    /// collections.OrderedDict: pair count u32, then (key id, value id) pairs
    /// in iteration order.
    OrderedDict = 0x21,
    /// collections.defaultdict: factory u8 (see `DEFAULT_FACTORIES`), pair
    /// count u32, then (key id, value id) pairs.
    DefaultDict = 0x22,
    /// collections.Counter: pair count u32, then (key id, value id) pairs.
    Counter = 0x23,
    /// collections.deque: maxlen u64 (0 = None, else maxlen + 1), count u32,
    /// then item ids.
    Deque = 0x24,
    /// Enum member: type id u16 (type table; no fields), mode u8 (0 = by
    /// name: child is a String record with the member name; 1 = by value),
    /// child id.
    Enum = 0x25,
    /// namedtuple instance: type id u16 (type table; fields = _fields),
    /// count u32, then item ids.
    NamedTuple = 0x26,
}

/// defaultdict factory codes (index = code). Code 0 is `None`.
pub const DEFAULT_FACTORIES: [&str; 12] = [
    "", "list", "dict", "set", "int", "float", "str", "tuple", "bytes", "frozenset", "bool", "complex",
];

impl Tag {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Tag::None),
            0x02 => Some(Tag::True),
            0x03 => Some(Tag::False),
            0x04 => Some(Tag::Int),
            0x05 => Some(Tag::Float),
            0x06 => Some(Tag::String),
            0x07 => Some(Tag::Bytes),
            0x08 => Some(Tag::List),
            0x09 => Some(Tag::Tuple),
            0x0A => Some(Tag::Dict),
            0x0B => Some(Tag::Set),
            0x0C => Some(Tag::FrozenSet),
            0x0D => Some(Tag::Dataclass),
            0x0E => Some(Tag::Reference),
            0x0F => Some(Tag::BigInt),
            0x10 => Some(Tag::ByteArray),
            0x11 => Some(Tag::Complex),
            0x12 => Some(Tag::StrRaw),
            0x13 => Some(Tag::DateTime),
            0x14 => Some(Tag::Date),
            0x15 => Some(Tag::Time),
            0x16 => Some(Tag::TimeDelta),
            0x17 => Some(Tag::Timezone),
            0x18 => Some(Tag::ZoneInfo),
            0x19 => Some(Tag::Decimal),
            0x1A => Some(Tag::Uuid),
            0x1B => Some(Tag::Fraction),
            0x1C => Some(Tag::Range),
            0x1D => Some(Tag::Slice),
            0x1E => Some(Tag::Ellipsis),
            0x1F => Some(Tag::NotImplemented),
            0x20 => Some(Tag::Path),
            0x21 => Some(Tag::OrderedDict),
            0x22 => Some(Tag::DefaultDict),
            0x23 => Some(Tag::Counter),
            0x24 => Some(Tag::Deque),
            0x25 => Some(Tag::Enum),
            0x26 => Some(Tag::NamedTuple),
            _ => None,
        }
    }
}

pub fn write_header(buf: &mut Vec<u8>, flags: u8) {
    buf.extend_from_slice(MAGIC_WRITE);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    buf.push(flags);
}

pub fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub fn write_i64_zigzag(buf: &mut Vec<u8>, v: i64) {
    let zigzag = ((v << 1) ^ (v >> 63)) as u64;
    write_u64_varint(buf, zigzag);
}

fn write_u64_varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

pub fn read_u32(data: &[u8], offset: &mut usize) -> Option<u32> {
    if *offset + 4 > data.len() {
        return None;
    }
    let v = u32::from_le_bytes(data[*offset..*offset + 4].try_into().ok()?);
    *offset += 4;
    Some(v)
}

pub fn read_u16(data: &[u8], offset: &mut usize) -> Option<u16> {
    if *offset + 2 > data.len() {
        return None;
    }
    let v = u16::from_le_bytes(data[*offset..*offset + 2].try_into().ok()?);
    *offset += 2;
    Some(v)
}

pub fn read_u8(data: &[u8], offset: &mut usize) -> Option<u8> {
    if *offset >= data.len() {
        return None;
    }
    let v = data[*offset];
    *offset += 1;
    Some(v)
}

pub fn read_i64_zigzag(data: &[u8], offset: &mut usize) -> Option<i64> {
    let zigzag = read_u64_varint(data, offset)?;
    Some(((zigzag >> 1) as i64) ^ -((zigzag & 1) as i64))
}

fn read_u64_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = read_u8(data, offset)?;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    Some(result)
}

pub fn read_bytes<'a>(data: &'a [u8], offset: &mut usize, len: usize) -> Option<&'a [u8]> {
    if *offset + len > data.len() {
        return None;
    }
    let slice = &data[*offset..*offset + len];
    *offset += len;
    Some(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zigzag_roundtrip() {
        let values: Vec<i64> = vec![0, 1, -1, 127, -128, 128, -129, i64::MAX, i64::MIN];
        for &v in &values {
            let mut buf = Vec::new();
            write_i64_zigzag(&mut buf, v);
            let mut offset = 0;
            let decoded = read_i64_zigzag(&mut buf, &mut offset).unwrap();
            assert_eq!(v, decoded, "failed for {}", v);
        }
    }

    #[test]
    fn test_header_roundtrip() {
        let mut buf = Vec::new();
        write_header(&mut buf, FLAG_HMAC);
        assert_eq!(&buf[0..4], MAGIC_WRITE);
        assert_eq!(u16::from_le_bytes(buf[4..6].try_into().unwrap()), FORMAT_VERSION);
        assert_eq!(u32::from_le_bytes(buf[6..10].try_into().unwrap()), SCHEMA_VERSION);
        assert_eq!(buf[10], FLAG_HMAC);
    }
}
