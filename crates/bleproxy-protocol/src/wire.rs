//! The Protocol Buffers wire format, limited to what the ESPHome API uses.
//!
//! Encoding goes through [`Encode`], which describes a message once and is
//! run against two sinks: a counter that computes the encoded length (needed
//! up front for nested messages and frame headers) and a buffer writer.
//! Decoding iterates over a message's [`Field`]s. Neither direction
//! allocates. Fields that hold their default value are omitted, as proto3
//! requires.

use crate::varint;

/// Errors produced while encoding a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The output buffer cannot hold the encoded message.
    #[error("buffer too small for the encoded message")]
    BufferTooSmall,
}

/// Errors produced while decoding a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The message ends in the middle of a field.
    #[error("message is truncated")]
    Truncated,
    /// A field key or varint is malformed.
    #[error("malformed field key or varint")]
    Malformed,
    /// A field uses a wire type this decoder does not support (groups).
    #[error("unsupported wire type {0}")]
    UnsupportedWireType(u8),
    /// A field has a different wire type than its definition.
    #[error("field {0} has an unexpected wire type")]
    UnexpectedWireType(u32),
    /// A field's value does not fit its type.
    #[error("field {0} is out of range")]
    OutOfRange(u32),
    /// A string field is not valid UTF-8.
    #[error("field {0} is not valid UTF-8")]
    InvalidUtf8(u32),
}

/// Destination for encoded bytes.
pub trait Sink {
    /// Appends `bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError::BufferTooSmall`] if there is no room left.
    fn put(&mut self, bytes: &[u8]) -> Result<(), EncodeError>;
}

/// A [`Sink`] that only counts bytes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counter(pub usize);

impl Sink for Counter {
    fn put(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        self.0 += bytes.len();
        Ok(())
    }
}

/// A [`Sink`] that writes into a fixed buffer.
#[derive(Debug)]
pub struct SliceSink<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SliceSink<'a> {
    /// Creates a sink that writes from the start of `buf`.
    pub const fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    /// Returns the number of bytes written so far.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if nothing has been written yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Sink for SliceSink<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self.len + bytes.len();
        self.buf
            .get_mut(self.len..end)
            .ok_or(EncodeError::BufferTooSmall)?
            .copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

/// Wire type of an encoded field. Nothing in the ESPHome API needs 64-bit
/// fixed-width fields, so only the decoder handles those.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireType {
    Varint = 0,
    Len = 2,
    Fixed32 = 5,
}

/// Writes protobuf fields to a [`Sink`].
#[derive(Debug)]
pub struct Encoder<S> {
    sink: S,
}

impl<S: Sink> Encoder<S> {
    /// Creates an encoder writing to `sink`.
    pub const fn new(sink: S) -> Self {
        Self { sink }
    }

    /// Returns the underlying sink.
    pub fn into_sink(self) -> S {
        self.sink
    }

    fn varint(&mut self, value: u64) -> Result<(), EncodeError> {
        let (bytes, len) = varint::encode(value);
        self.sink.put(bytes.get(..len).unwrap_or_default())
    }

    fn key(&mut self, field: u32, wire_type: WireType) -> Result<(), EncodeError> {
        self.varint(u64::from(field) << 3 | wire_type as u64)
    }

    /// Writes a `uint32` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn uint32(&mut self, field: u32, value: u32) -> Result<(), EncodeError> {
        self.uint64(field, value.into())
    }

    /// Writes a `uint64` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn uint64(&mut self, field: u32, value: u64) -> Result<(), EncodeError> {
        if value == 0 {
            return Ok(());
        }
        self.key(field, WireType::Varint)?;
        self.varint(value)
    }

    /// Writes an `int32` field. Negative values take ten bytes, since they
    /// are sign-extended to 64 bits.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn int32(&mut self, field: u32, value: i32) -> Result<(), EncodeError> {
        self.uint64(field, i64::from(value).cast_unsigned())
    }

    /// Writes a `repeated uint64` field, packed as proto3 does by default.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn packed_uint64(&mut self, field: u32, values: &[u64]) -> Result<(), EncodeError> {
        if values.is_empty() {
            return Ok(());
        }
        let len: usize = values.iter().map(|value| varint::encode(*value).1).sum();
        self.key(field, WireType::Len)?;
        self.varint(len as u64)?;
        values.iter().try_for_each(|value| self.varint(*value))
    }

    /// Writes a `sint32` field (zigzag encoded).
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn sint32(&mut self, field: u32, value: i32) -> Result<(), EncodeError> {
        let zigzag = (value << 1) ^ (value >> 31);
        self.uint32(field, zigzag.cast_unsigned())
    }

    /// Writes a `bool` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn bool(&mut self, field: u32, value: bool) -> Result<(), EncodeError> {
        self.uint64(field, value.into())
    }

    /// Writes a `fixed32` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn fixed32(&mut self, field: u32, value: u32) -> Result<(), EncodeError> {
        if value == 0 {
            return Ok(());
        }
        self.key(field, WireType::Fixed32)?;
        self.sink.put(&value.to_le_bytes())
    }

    /// Writes a `float` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn float(&mut self, field: u32, value: f32) -> Result<(), EncodeError> {
        self.fixed32(field, value.to_bits())
    }

    /// Writes a `string` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn string(&mut self, field: u32, value: &str) -> Result<(), EncodeError> {
        self.bytes(field, value.as_bytes())
    }

    /// Writes a `bytes` field.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn bytes(&mut self, field: u32, value: &[u8]) -> Result<(), EncodeError> {
        if value.is_empty() {
            return Ok(());
        }
        self.key(field, WireType::Len)?;
        self.varint(value.len() as u64)?;
        self.sink.put(value)
    }

    /// Writes an embedded message field.
    ///
    /// Unlike scalar fields, embedded messages are written even when empty,
    /// since presence is meaningful for them (for example as elements of a
    /// repeated field).
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    pub fn message<M: Encode + ?Sized>(
        &mut self,
        field: u32,
        value: &M,
    ) -> Result<(), EncodeError> {
        self.key(field, WireType::Len)?;
        self.varint(value.encoded_len() as u64)?;
        value.encode(self)
    }
}

/// A protobuf message that can be encoded.
pub trait Encode {
    /// Writes the message's fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is full.
    fn encode<S: Sink>(&self, encoder: &mut Encoder<S>) -> Result<(), EncodeError>;

    /// Returns the length of the encoded message in bytes.
    fn encoded_len(&self) -> usize {
        let mut encoder = Encoder::new(Counter::default());
        // Counting never fails.
        let _ = self.encode(&mut encoder);
        encoder.into_sink().0
    }

    /// Encodes the message to the start of `buf`, returning its length.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError::BufferTooSmall`] if `buf` is too small.
    fn encode_to_slice(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let mut encoder = Encoder::new(SliceSink::new(buf));
        self.encode(&mut encoder)?;
        Ok(encoder.into_sink().len())
    }
}

/// A decoded field value, still in its wire representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value<'a> {
    /// A varint (`int32`, `uint32`, `uint64`, `sint32`, `bool`, enums...).
    Varint(u64),
    /// A 64-bit fixed-width value.
    Fixed64(u64),
    /// A length-delimited value (`string`, `bytes`, embedded messages).
    Len(&'a [u8]),
    /// A 32-bit fixed-width value.
    Fixed32(u32),
}

/// A decoded field: its number and value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<'a> {
    /// The field number.
    pub number: u32,
    /// The field value.
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    /// Interprets the field as a `uint32`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not a varint or does not fit.
    pub fn as_u32(&self) -> Result<u32, DecodeError> {
        u32::try_from(self.as_u64()?).map_err(|_| DecodeError::OutOfRange(self.number))
    }

    /// Interprets the field as a `fixed32`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not a 32-bit value.
    pub const fn as_fixed32(&self) -> Result<u32, DecodeError> {
        match self.value {
            Value::Fixed32(value) => Ok(value),
            _ => Err(DecodeError::UnexpectedWireType(self.number)),
        }
    }

    /// Interprets the field as a `uint64`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not a varint.
    pub const fn as_u64(&self) -> Result<u64, DecodeError> {
        match self.value {
            Value::Varint(value) => Ok(value),
            _ => Err(DecodeError::UnexpectedWireType(self.number)),
        }
    }

    /// Interprets the field as a `bool`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not a varint.
    pub const fn as_bool(&self) -> Result<bool, DecodeError> {
        match self.as_u64() {
            Ok(value) => Ok(value != 0),
            Err(err) => Err(err),
        }
    }

    /// Interprets the field as `bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not length-delimited.
    pub const fn as_bytes(&self) -> Result<&'a [u8], DecodeError> {
        match self.value {
            Value::Len(bytes) => Ok(bytes),
            _ => Err(DecodeError::UnexpectedWireType(self.number)),
        }
    }

    /// Interprets the field as a `string`.
    ///
    /// # Errors
    ///
    /// Returns an error if the field is not length-delimited or not UTF-8.
    pub fn as_str(&self) -> Result<&'a str, DecodeError> {
        core::str::from_utf8(self.as_bytes()?).map_err(|_| DecodeError::InvalidUtf8(self.number))
    }
}

/// Iterates over the fields of an encoded message.
///
/// ```
/// use bleproxy_protocol::wire::{Fields, Value};
///
/// // Field 1 = 150 (varint), field 2 = "hi" (length-delimited).
/// let mut fields = Fields::new(&[0x08, 0x96, 0x01, 0x12, 0x02, b'h', b'i']);
/// let first = fields.next().unwrap()?;
/// assert_eq!((first.number, first.value), (1, Value::Varint(150)));
/// assert_eq!(fields.next().unwrap()?.as_str()?, "hi");
/// assert!(fields.next().is_none());
/// # Ok::<(), bleproxy_protocol::wire::DecodeError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Fields<'a> {
    buf: &'a [u8],
}

impl<'a> Fields<'a> {
    /// Creates an iterator over the fields encoded in `buf`.
    #[must_use]
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    fn varint(&mut self) -> Result<u64, DecodeError> {
        let (value, len) = varint::decode_u64(self.buf)
            .map_err(|varint::Overflow| DecodeError::Malformed)?
            .ok_or(DecodeError::Truncated)?;
        self.buf = self.buf.get(len..).unwrap_or_default();
        Ok(value)
    }

    const fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if len > self.buf.len() {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.buf.split_at(len);
        self.buf = tail;
        Ok(head)
    }

    fn next_field(&mut self) -> Result<Field<'a>, DecodeError> {
        let key = self.varint()?;
        let number = u32::try_from(key >> 3).map_err(|_| DecodeError::Malformed)?;
        if number == 0 {
            return Err(DecodeError::Malformed);
        }
        let value = match key & 0x07 {
            0 => Value::Varint(self.varint()?),
            1 => Value::Fixed64(u64::from_le_bytes(
                self.take(8)?
                    .try_into()
                    .map_err(|_| DecodeError::Truncated)?,
            )),
            2 => {
                let len = usize::try_from(self.varint()?).map_err(|_| DecodeError::Truncated)?;
                Value::Len(self.take(len)?)
            }
            5 => Value::Fixed32(u32::from_le_bytes(
                self.take(4)?
                    .try_into()
                    .map_err(|_| DecodeError::Truncated)?,
            )),
            // `key & 0x07` always fits in a u8.
            other => return Err(DecodeError::UnsupportedWireType(other.to_le_bytes()[0])),
        };
        Ok(Field { number, value })
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<Field<'a>, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let field = self.next_field();
        if field.is_err() {
            // Stop after the first error instead of reading garbage.
            self.buf = &[];
        }
        Some(field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sample<'a> {
        number: u32,
        big: u64,
        signed: i32,
        flag: bool,
        fixed: u32,
        text: &'a str,
        nested: Option<Inner>,
    }

    struct Inner(u32);

    impl Encode for Inner {
        fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
            e.uint32(1, self.0)
        }
    }

    impl Encode for Sample<'_> {
        fn encode<S: Sink>(&self, e: &mut Encoder<S>) -> Result<(), EncodeError> {
            e.uint32(1, self.number)?;
            e.uint64(2, self.big)?;
            e.sint32(3, self.signed)?;
            e.bool(4, self.flag)?;
            e.fixed32(5, self.fixed)?;
            e.string(6, self.text)?;
            if let Some(nested) = &self.nested {
                e.message(7, nested)?;
            }
            Ok(())
        }
    }

    fn encode(message: &impl Encode) -> ([u8; 64], usize) {
        let mut buf = [0; 64];
        let len = message.encode_to_slice(&mut buf).unwrap();
        assert_eq!(len, message.encoded_len());
        (buf, len)
    }

    #[test]
    fn omits_default_scalars() {
        let empty = Sample {
            number: 0,
            big: 0,
            signed: 0,
            flag: false,
            fixed: 0,
            text: "",
            nested: None,
        };
        assert_eq!(encode(&empty).1, 0);
    }

    #[test]
    fn encodes_every_field_type() {
        let sample = Sample {
            number: 150,
            big: 0xA4C1_3800_1122,
            signed: -70,
            flag: true,
            fixed: 0x0102_0304,
            text: "hi",
            nested: Some(Inner(0)),
        };
        let (buf, len) = encode(&sample);
        assert_eq!(
            &buf[..len],
            [
                0x08, 0x96, 0x01, // 1: 150
                0x10, 0xA2, 0xA2, 0x80, 0xC0, 0x93, 0x98, 0x29, // 2: address
                0x18, 0x8B, 0x01, // 3: zigzag(-70) = 139
                0x20, 0x01, // 4: true
                0x2D, 0x04, 0x03, 0x02, 0x01, // 5: fixed32
                0x32, 0x02, b'h', b'i', // 6: "hi"
                0x3A, 0x00, // 7: empty nested message, still present
            ]
        );
    }

    #[test]
    fn zigzag_extremes() {
        for (value, expected) in [(0, 0), (-1, 1), (1, 2), (i32::MIN, u32::MAX)] {
            let mut buf = [0; 8];
            let mut e = Encoder::new(SliceSink::new(&mut buf));
            e.sint32(1, value).unwrap();
            let len = e.into_sink().len();
            let field = Fields::new(&buf[..len]).next().map(Result::unwrap);
            // Zero is the default value, so it is not encoded at all.
            assert_eq!(field.map_or(0, |f| f.as_u32().unwrap()), expected);
        }
    }

    #[test]
    fn reports_short_buffers() {
        let sample = Sample {
            number: 1,
            big: 0,
            signed: 0,
            flag: false,
            fixed: 0,
            text: "hello",
            nested: None,
        };
        let len = sample.encoded_len();
        for short in 0..len {
            assert_eq!(
                sample.encode_to_slice(&mut [0; 16][..short]),
                Err(EncodeError::BufferTooSmall)
            );
        }
    }

    #[test]
    fn encodes_negative_int32_and_packed_fields() {
        let mut buf = [0; 32];
        let mut e = Encoder::new(SliceSink::new(&mut buf));
        e.int32(1, -1).unwrap();
        e.int32(2, 5).unwrap();
        e.packed_uint64(3, &[1, 300]).unwrap();
        e.packed_uint64(4, &[]).unwrap();
        let len = e.into_sink().len();
        assert_eq!(
            &buf[..len],
            [
                0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, // 1: -1
                0x10, 0x05, // 2: 5
                0x1A, 0x03, 0x01, 0xAC, 0x02, // 3: [1, 300]
            ]
        );
    }

    #[test]
    fn decodes_every_wire_type() {
        let buf = [
            0x08, 0x96, 0x01, // 1: varint 150
            0x11, 1, 0, 0, 0, 0, 0, 0, 0, // 2: fixed64 1
            0x1A, 0x02, b'h', b'i', // 3: "hi"
            0x25, 4, 3, 2, 1, // 4: fixed32
        ];
        let fields: [Field<'_>; 4] = {
            let mut it = Fields::new(&buf).map(Result::unwrap);
            core::array::from_fn(|_| it.next().unwrap())
        };
        assert_eq!(fields[0].value, Value::Varint(150));
        assert_eq!(fields[1].value, Value::Fixed64(1));
        assert_eq!(fields[2].as_str(), Ok("hi"));
        assert_eq!(fields[3].value, Value::Fixed32(0x0102_0304));
    }

    #[test]
    fn rejects_malformed_input() {
        let cases: [(&[u8], DecodeError); 6] = [
            (&[0x08], DecodeError::Truncated),
            (&[0x12, 0x05, b'a'], DecodeError::Truncated),
            (&[0x15, 0x01], DecodeError::Truncated),
            (&[0x00, 0x00], DecodeError::Malformed),
            (&[0x0B], DecodeError::UnsupportedWireType(3)),
            (&[0x0A, 0x01, 0xFF], DecodeError::InvalidUtf8(1)),
        ];
        for (input, expected) in cases {
            let result = Fields::new(input)
                .next()
                .unwrap()
                .and_then(|field| field.as_str().map(|_| ()));
            assert_eq!(result, Err(expected), "{input:02x?}");
        }
    }

    #[test]
    fn stops_after_an_error() {
        let mut fields = Fields::new(&[0x08]);
        assert!(fields.next().unwrap().is_err());
        assert!(fields.next().is_none());
    }

    #[test]
    fn type_checks_fields() {
        let field = Field {
            number: 3,
            value: Value::Len(b"x"),
        };
        assert_eq!(field.as_u32(), Err(DecodeError::UnexpectedWireType(3)));
        let field = Field {
            number: 4,
            value: Value::Varint(u64::from(u32::MAX) + 1),
        };
        assert_eq!(field.as_u32(), Err(DecodeError::OutOfRange(4)));
        assert_eq!(field.as_bytes(), Err(DecodeError::UnexpectedWireType(4)));
    }
}
