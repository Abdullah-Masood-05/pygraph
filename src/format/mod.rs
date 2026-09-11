pub mod encoder;
pub mod decoder;

pub const MAGIC: &[u8; 4] = b"PYGR";
pub const FORMAT_VERSION: u16 = 1;
pub const SCHEMA_VERSION: u32 = 0;
pub const FLAG_HMAC: u8 = 1 << 0;
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
}

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
            _ => None,
        }
    }
}

pub fn write_header(buf: &mut Vec<u8>, flags: u8) {
    buf.extend_from_slice(MAGIC);
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
        assert_eq!(&buf[0..4], MAGIC);
        assert_eq!(u16::from_le_bytes(buf[4..6].try_into().unwrap()), FORMAT_VERSION);
        assert_eq!(u32::from_le_bytes(buf[6..10].try_into().unwrap()), SCHEMA_VERSION);
        assert_eq!(buf[10], FLAG_HMAC);
    }
}
