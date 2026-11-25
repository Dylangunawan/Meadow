//! Minimal, fast NBT implementation (Java edition, big-endian).
//!
//! Compounds keep insertion order, which keeps written files diff-friendly and makes
//! output deterministic. Strings use Java's "modified UTF-8".

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

#[derive(Debug, thiserror::Error)]
pub enum NbtError {
    #[error("unexpected end of NBT data")]
    Eof,
    #[error("invalid tag type {0}")]
    BadTag(u8),
    #[error("invalid string data")]
    BadString,
    #[error("root tag is not a compound")]
    NotCompound,
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, NbtError>;

pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A homogeneous list. `elem` is the element tag type (TAG_END for empty lists).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct List {
    pub elem: u8,
    pub items: Vec<Tag>,
}

impl List {
    pub fn new() -> Self {
        Self { elem: TAG_END, items: Vec::new() }
    }
    pub fn of(items: Vec<Tag>) -> Self {
        let elem = items.first().map(Tag::id).unwrap_or(TAG_END);
        Self { elem, items }
    }
    pub fn push(&mut self, t: impl Into<Tag>) {
        let t = t.into();
        if self.items.is_empty() {
            self.elem = t.id();
        }
        self.items.push(t);
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Tag> {
        self.items.iter()
    }
}

/// Ordered compound tag.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    entries: Vec<(String, Tag)>,
}

impl Compound {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }
    pub fn with_capacity(n: usize) -> Self {
        Self { entries: Vec::with_capacity(n) }
    }
    /// Insert or replace a value.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Tag>) -> &mut Self {
        let key = key.into();
        let value = value.into();
        if let Some(e) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            e.1 = value;
        } else {
            self.entries.push((key, value));
        }
        self
    }
    /// Builder-style insert.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Tag>) -> Self {
        self.insert(key, value);
        self
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Tag)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // Typed accessors. Numeric getters accept any numeric tag type.
    pub fn compound(&self, key: &str) -> Option<&Compound> {
        match self.get(key) {
            Some(Tag::Compound(c)) => Some(c),
            _ => None,
        }
    }
    pub fn compound_mut(&mut self, key: &str) -> Option<&mut Compound> {
        match self.get_mut(key) {
            Some(Tag::Compound(c)) => Some(c),
            _ => None,
        }
    }
    pub fn list(&self, key: &str) -> Option<&List> {
        match self.get(key) {
            Some(Tag::List(l)) => Some(l),
            _ => None,
        }
    }
    pub fn string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Tag::String(s)) => Some(s),
            _ => None,
        }
    }
    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Tag::as_i64)
    }
    pub fn i32(&self, key: &str) -> Option<i32> {
        self.i64(key).map(|v| v as i32)
    }
    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Tag::as_f64)
    }
    pub fn byte_array(&self, key: &str) -> Option<&[i8]> {
        match self.get(key) {
            Some(Tag::ByteArray(v)) => Some(v),
            _ => None,
        }
    }
    pub fn int_array(&self, key: &str) -> Option<&[i32]> {
        match self.get(key) {
            Some(Tag::IntArray(v)) => Some(v),
            _ => None,
        }
    }
    pub fn long_array(&self, key: &str) -> Option<&[i64]> {
        match self.get(key) {
            Some(Tag::LongArray(v)) => Some(v),
            _ => None,
        }
    }
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => TAG_BYTE,
            Tag::Short(_) => TAG_SHORT,
            Tag::Int(_) => TAG_INT,
            Tag::Long(_) => TAG_LONG,
            Tag::Float(_) => TAG_FLOAT,
            Tag::Double(_) => TAG_DOUBLE,
            Tag::ByteArray(_) => TAG_BYTE_ARRAY,
            Tag::String(_) => TAG_STRING,
            Tag::List(_) => TAG_LIST,
            Tag::Compound(_) => TAG_COMPOUND,
            Tag::IntArray(_) => TAG_INT_ARRAY,
            Tag::LongArray(_) => TAG_LONG_ARRAY,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Tag::Byte(v) => v as i64,
            Tag::Short(v) => v as i64,
            Tag::Int(v) => v as i64,
            Tag::Long(v) => v,
            Tag::Float(v) => v as i64,
            Tag::Double(v) => v as i64,
            _ => return None,
        })
    }
    pub fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Tag::Byte(v) => v as f64,
            Tag::Short(v) => v as f64,
            Tag::Int(v) => v as f64,
            Tag::Long(v) => v as f64,
            Tag::Float(v) => v as f64,
            Tag::Double(v) => v,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&List> {
        match self {
            Tag::List(l) => Some(l),
            _ => None,
        }
    }
}

macro_rules! from_impl {
    ($t:ty, $v:ident) => {
        impl From<$t> for Tag {
            fn from(x: $t) -> Tag {
                Tag::$v(x)
            }
        }
    };
}
from_impl!(i8, Byte);
from_impl!(i16, Short);
from_impl!(i32, Int);
from_impl!(i64, Long);
from_impl!(f32, Float);
from_impl!(f64, Double);
from_impl!(String, String);
from_impl!(Compound, Compound);
from_impl!(List, List);
from_impl!(Vec<i8>, ByteArray);
from_impl!(Vec<i32>, IntArray);
from_impl!(Vec<i64>, LongArray);

impl From<&str> for Tag {
    fn from(s: &str) -> Tag {
        Tag::String(s.to_owned())
    }
}
impl From<bool> for Tag {
    fn from(b: bool) -> Tag {
        Tag::Byte(b as i8)
    }
}

// ---------------------------------------------------------------------------------------------
// Writing

/// Append-only big-endian NBT writer.
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn with_capacity(n: usize) -> Self {
        Self { buf: Vec::with_capacity(n) }
    }
    #[inline]
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    #[inline]
    fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    #[inline]
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn string(&mut self, s: &str) {
        if s.bytes().all(|b| b != 0 && b < 0x80) {
            self.buf.extend_from_slice(&(s.len() as u16).to_be_bytes());
            self.buf.extend_from_slice(s.as_bytes());
        } else {
            let enc = encode_mutf8(s);
            self.buf.extend_from_slice(&(enc.len() as u16).to_be_bytes());
            self.buf.extend_from_slice(&enc);
        }
    }
    fn payload(&mut self, t: &Tag) {
        match t {
            Tag::Byte(v) => self.u8(*v as u8),
            Tag::Short(v) => self.i16(*v),
            Tag::Int(v) => self.i32(*v),
            Tag::Long(v) => self.buf.extend_from_slice(&v.to_be_bytes()),
            Tag::Float(v) => self.buf.extend_from_slice(&v.to_be_bytes()),
            Tag::Double(v) => self.buf.extend_from_slice(&v.to_be_bytes()),
            Tag::ByteArray(v) => {
                self.i32(v.len() as i32);
                self.buf.extend(v.iter().map(|b| *b as u8));
            }
            Tag::String(s) => self.string(s),
            Tag::List(l) => {
                let elem = if l.items.is_empty() { TAG_END } else { l.elem };
                self.u8(elem);
                self.i32(l.items.len() as i32);
                for i in &l.items {
                    self.payload(i);
                }
            }
            Tag::Compound(c) => self.compound_body(c),
            Tag::IntArray(v) => {
                self.i32(v.len() as i32);
                self.buf.reserve(v.len() * 4);
                for x in v {
                    self.buf.extend_from_slice(&x.to_be_bytes());
                }
            }
            Tag::LongArray(v) => {
                self.i32(v.len() as i32);
                self.buf.reserve(v.len() * 8);
                for x in v {
                    self.buf.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }
    fn compound_body(&mut self, c: &Compound) {
        for (k, v) in &c.entries {
            self.u8(v.id());
            self.string(k);
            self.payload(v);
        }
        self.u8(TAG_END);
    }
}

/// Serialise a root compound (with name) to uncompressed bytes.
pub fn to_bytes(root_name: &str, root: &Compound) -> Vec<u8> {
    let mut w = Writer::with_capacity(4096);
    w.u8(TAG_COMPOUND);
    w.string(root_name);
    w.compound_body(root);
    w.buf
}

pub fn to_gzip(root_name: &str, root: &Compound) -> std::io::Result<Vec<u8>> {
    let raw = to_bytes(root_name, root);
    let mut e = GzEncoder::new(Vec::with_capacity(raw.len() / 4), Compression::default());
    e.write_all(&raw)?;
    e.finish()
}

pub fn to_zlib(root_name: &str, root: &Compound, level: u32) -> std::io::Result<Vec<u8>> {
    let raw = to_bytes(root_name, root);
    let mut e = ZlibEncoder::new(Vec::with_capacity(raw.len() / 4), Compression::new(level));
    e.write_all(&raw)?;
    e.finish()
}

// ---------------------------------------------------------------------------------------------
// Reading

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    #[inline]
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(NbtError::Eof);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    #[inline]
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    #[inline]
    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    #[inline]
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    #[inline]
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn len(&mut self) -> Result<usize> {
        let n = self.i32()?;
        Ok(n.max(0) as usize)
    }
    fn string(&mut self) -> Result<String> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        let b = self.take(n)?;
        match std::str::from_utf8(b) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => decode_mutf8(b).ok_or(NbtError::BadString),
        }
    }
    fn payload(&mut self, id: u8, depth: u32) -> Result<Tag> {
        if depth > 512 {
            return Err(NbtError::BadTag(id));
        }
        Ok(match id {
            TAG_BYTE => Tag::Byte(self.u8()? as i8),
            TAG_SHORT => Tag::Short(self.i16()?),
            TAG_INT => Tag::Int(self.i32()?),
            TAG_LONG => Tag::Long(self.i64()?),
            TAG_FLOAT => Tag::Float(f32::from_bits(self.i32()? as u32)),
            TAG_DOUBLE => Tag::Double(f64::from_bits(self.i64()? as u64)),
            TAG_BYTE_ARRAY => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.iter().map(|b| *b as i8).collect())
            }
            TAG_STRING => Tag::String(self.string()?),
            TAG_LIST => {
                let elem = self.u8()?;
                let n = self.len()?;
                if n > self.data.len() {
                    return Err(NbtError::Eof);
                }
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.payload(elem, depth + 1)?);
                }
                Tag::List(List { elem, items })
            }
            TAG_COMPOUND => Tag::Compound(self.compound_body(depth + 1)?),
            TAG_INT_ARRAY => {
                let n = self.len()?;
                let b = self.take(n * 4)?;
                Tag::IntArray(b.chunks_exact(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())).collect())
            }
            TAG_LONG_ARRAY => {
                let n = self.len()?;
                let b = self.take(n * 8)?;
                Tag::LongArray(b.chunks_exact(8).map(|c| i64::from_be_bytes(c.try_into().unwrap())).collect())
            }
            other => return Err(NbtError::BadTag(other)),
        })
    }
    fn compound_body(&mut self, depth: u32) -> Result<Compound> {
        let mut c = Compound::new();
        loop {
            let id = self.u8()?;
            if id == TAG_END {
                return Ok(c);
            }
            let name = self.string()?;
            let v = self.payload(id, depth)?;
            c.entries.push((name, v));
        }
    }
}

/// Parse uncompressed NBT. Returns (root name, root compound).
pub fn from_bytes(data: &[u8]) -> Result<(String, Compound)> {
    let mut r = Reader { data, pos: 0 };
    let id = r.u8()?;
    if id != TAG_COMPOUND {
        return Err(NbtError::NotCompound);
    }
    let name = r.string()?;
    let c = r.compound_body(0)?;
    Ok((name, c))
}

/// Parse NBT that may be gzip-, zlib- or un-compressed (auto-detected).
pub fn from_any(data: &[u8]) -> Result<(String, Compound)> {
    let raw = decompress_any(data)?;
    from_bytes(&raw)
}

/// Decompress gzip or zlib data, or return the input unchanged if it is neither.
pub fn decompress_any(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4);
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        GzDecoder::new(data).read_to_end(&mut out)?;
    } else if data.len() >= 2 && data[0] == 0x78 {
        ZlibDecoder::new(data).read_to_end(&mut out)?;
    } else {
        out.extend_from_slice(data);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Modified UTF-8

fn encode_mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 8);
    for u in s.encode_utf16() {
        match u {
            0x0001..=0x007f => out.push(u as u8),
            0x0000 | 0x0080..=0x07ff => {
                out.push(0xc0 | ((u >> 6) & 0x1f) as u8);
                out.push(0x80 | (u & 0x3f) as u8);
            }
            _ => {
                out.push(0xe0 | ((u >> 12) & 0x0f) as u8);
                out.push(0x80 | ((u >> 6) & 0x3f) as u8);
                out.push(0x80 | (u & 0x3f) as u8);
            }
        }
    }
    out
}

fn decode_mutf8(b: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as u16;
        if c & 0x80 == 0 {
            units.push(c);
            i += 1;
        } else if c & 0xe0 == 0xc0 {
            let c2 = *b.get(i + 1)? as u16;
            units.push(((c & 0x1f) << 6) | (c2 & 0x3f));
            i += 2;
        } else if c & 0xf0 == 0xe0 {
            let c2 = *b.get(i + 1)? as u16;
            let c3 = *b.get(i + 2)? as u16;
            units.push(((c & 0x0f) << 12) | ((c2 & 0x3f) << 6) | (c3 & 0x3f));
            i += 3;
        } else {
            return None;
        }
    }
    Some(String::from_utf16_lossy(&units))
}

/// Pretty-print a tag as SNBT-like text (debugging aid).
pub fn to_snbt(t: &Tag) -> String {
    let mut s = String::new();
    snbt(t, &mut s);
    s
}

fn snbt(t: &Tag, out: &mut String) {
    use std::fmt::Write as _;
    match t {
        Tag::Byte(v) => write!(out, "{v}b").unwrap(),
        Tag::Short(v) => write!(out, "{v}s").unwrap(),
        Tag::Int(v) => write!(out, "{v}").unwrap(),
        Tag::Long(v) => write!(out, "{v}L").unwrap(),
        Tag::Float(v) => write!(out, "{v}f").unwrap(),
        Tag::Double(v) => write!(out, "{v}d").unwrap(),
        Tag::ByteArray(v) => write!(out, "[B;{} bytes]", v.len()).unwrap(),
        Tag::String(v) => write!(out, "{v:?}").unwrap(),
        Tag::List(l) => {
            out.push('[');
            for (i, x) in l.items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                snbt(x, out);
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            out.push('{');
            for (i, (k, v)) in c.entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write!(out, "{k}:").unwrap();
                snbt(v, out);
            }
            out.push('}');
        }
        Tag::IntArray(v) => write!(out, "[I;{} ints]", v.len()).unwrap(),
        Tag::LongArray(v) => write!(out, "[L;{} longs]", v.len()).unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut list = List::new();
        list.push(Tag::Int(1));
        list.push(Tag::Int(2));
        let c = Compound::new()
            .with("byte", 3i8)
            .with("name", "héllo\0wörld 😀")
            .with("longs", vec![1i64, -2, i64::MAX])
            .with("list", list)
            .with("empty", List::new())
            .with("nested", Compound::new().with("d", 1.5f64));
        let bytes = to_gzip("root", &c).unwrap();
        let (name, back) = from_any(&bytes).unwrap();
        assert_eq!(name, "root");
        assert_eq!(back, c);
        assert_eq!(back.string("name"), Some("héllo\0wörld 😀"));
        assert_eq!(back.compound("nested").unwrap().f64("d"), Some(1.5));
    }
}
