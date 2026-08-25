//! Structural checksum folding shared by gameplay observation and history
//! logs.
//!
//! Every `serde` callback writes a distinct type tag followed by fixed-width
//! little-endian payload bytes into an FNV-1a fold, so two values collide
//! only when their serialized shapes, field names, ordering, and contents all
//! match. This keeps the guarantees — stable ordering in, stable checksum
//! out, sensitive to any visited field — while removing number formatting,
//! string escaping, and JSON framing from observation paths.
//!
//! Serialization of observed state is infallible, so the error slot exists
//! only to satisfy the [`serde::ser::Serializer`] trait.

use serde::Serialize;
use serde::ser;

/// Serialization error slot for the structural checksum folder.
///
/// Serialization of gameplay observation state is infallible, so this type is
/// never constructed; it exists only to satisfy the [`Serializer`] trait.
#[derive(Debug, Default)]
pub(crate) struct ChecksumError;

impl std::fmt::Display for ChecksumError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("structural checksum serialization cannot fail")
    }
}

impl ser::StdError for ChecksumError {}

impl ser::Error for ChecksumError {
    fn custom<T: std::fmt::Display>(_message: T) -> Self {
        Self
    }
}

/// Serializes the serde data model into a running FNV-1a fold.
///
/// The fold is a plain `u64`, so it can be snapshotted and resumed: folding
/// entries one at a time from a saved mid-state produces exactly the same
/// result as serializing the whole sequence in one pass. [`HistoryLog`]
/// exploits that to maintain its entry-stream checksum incrementally.
pub(crate) struct ChecksumFolder {
    state: u64,
}

impl ChecksumFolder {
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;

    pub(crate) fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    /// Resumes a fold from a mid-state captured by [`Self::raw`].
    pub(crate) fn from_raw(state: u64) -> Self {
        Self { state }
    }

    /// Returns the running fold so it can be resumed later.
    pub(crate) const fn raw(&self) -> u64 {
        self.state
    }

    pub(crate) fn finish(self) -> u64 {
        self.state
    }

    /// Folds the entry count into the running fold and returns the final
    /// value, so histories that differ only in length stay distinct.
    pub(crate) fn finish_with_entry_count(mut self, count: usize) -> u64 {
        self.tagged_bytes(
            b'#',
            &u64::try_from(count).unwrap_or(u64::MAX).to_le_bytes(),
        );
        self.state
    }

    fn byte(&mut self, tag: u8) {
        self.state = self.state.wrapping_mul(Self::PRIME) ^ u64::from(tag);
    }

    fn word(&mut self, value: u64) {
        self.state = (self.state ^ value).wrapping_mul(Self::PRIME);
    }

    fn tagged_bytes<const N: usize>(&mut self, tag: u8, payload: &[u8; N]) {
        self.byte(tag);
        self.fold_bytes(payload);
    }

    /// Folds raw bytes one machine word at a time. Callers always precede
    /// variable-length blobs with an explicit length fold, so zero padding of
    /// the final partial word cannot merge two different shapes.
    fn fold_bytes(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            #[allow(clippy::expect_used)] // chunks_exact guarantees eight bytes
            let array: [u8; 8] = chunk
                .try_into()
                .expect("chunks_exact yields exactly eight bytes");
            self.word(u64::from_le_bytes(array));
        }
        let remainder = chunks.remainder();
        if !remainder.is_empty() {
            let mut last = [0_u8; 8];
            last[..remainder.len()].copy_from_slice(remainder);
            self.word(u64::from_le_bytes(last));
        }
    }

    fn length(&mut self, tag: u8, length: usize) {
        self.tagged_bytes(
            tag,
            &u64::try_from(length).unwrap_or(u64::MAX).to_le_bytes(),
        );
    }

    fn int<const N: usize>(&mut self, tag: u8, payload: &[u8; N]) {
        self.tagged_bytes(tag, payload);
    }

    fn seq_length(&mut self, len: usize) {
        self.length(b'[', len);
    }

    fn fold_str(&mut self, value: &str) {
        self.length(b'S', value.len());
        self.fold_bytes(value.as_bytes());
        // A terminator keeps "ab" + "c" distinct from "a" + "bc".
        self.byte(0xFF);
    }
}

impl Default for ChecksumFolder {
    fn default() -> Self {
        Self::new()
    }
}

impl ser::Serializer for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, value: bool) -> Result<(), Self::Error> {
        self.byte(if value { b'T' } else { b'F' });
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), Self::Error> {
        self.int(b'b', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_i16(self, value: i16) -> Result<(), Self::Error> {
        self.int(b'h', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_i32(self, value: i32) -> Result<(), Self::Error> {
        self.int(b'i', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_i64(self, value: i64) -> Result<(), Self::Error> {
        self.int(b'l', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_u8(self, value: u8) -> Result<(), Self::Error> {
        self.int(b'B', &[value]);
        Ok(())
    }

    fn serialize_u16(self, value: u16) -> Result<(), Self::Error> {
        self.int(b'H', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_u32(self, value: u32) -> Result<(), Self::Error> {
        self.int(b'I', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_u64(self, value: u64) -> Result<(), Self::Error> {
        self.int(b'L', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_i128(self, value: i128) -> Result<(), Self::Error> {
        self.int(b'm', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_u128(self, value: u128) -> Result<(), Self::Error> {
        self.int(b'M', &value.to_le_bytes());
        Ok(())
    }

    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        self.int(b'f', &value.to_bits().to_le_bytes());
        Ok(())
    }

    fn serialize_f64(self, value: f64) -> Result<(), Self::Error> {
        self.int(b'd', &value.to_bits().to_le_bytes());
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<(), Self::Error> {
        self.int(b'c', &u32::from(value).to_le_bytes());
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<(), Self::Error> {
        self.fold_str(value);
        Ok(())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), Self::Error> {
        self.length(b'Y', value.len());
        self.fold_bytes(value);
        self.byte(0xFF);
        Ok(())
    }

    fn serialize_none(self) -> Result<(), Self::Error> {
        self.byte(b'0');
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        self.byte(b'1');
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.byte(b'u');
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        self.byte(b'u');
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.int(b'V', &variant_index.to_le_bytes());
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.byte(b'n');
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.byte(b'N');
        self.int(b'V', &variant_index.to_le_bytes());
        value.serialize(self)
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        if let Some(len) = len {
            self.seq_length(len);
        } else {
            self.byte(b'?');
        }
        Ok(self)
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.seq_length(len);
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.seq_length(len);
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.byte(b't');
        self.int(b'V', &variant_index.to_le_bytes());
        self.length(b'[', len);
        Ok(self)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        if let Some(len) = len {
            self.length(b'{', len);
        } else {
            self.byte(b'?');
        }
        Ok(self)
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.serialize_str(name)?;
        self.length(b'{', len);
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.byte(b's');
        self.int(b'V', &variant_index.to_le_bytes());
        self.length(b'{', len);
        Ok(self)
    }
}

impl ser::SerializeSeq for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b']');
        Ok(())
    }
}

impl ser::SerializeTuple for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b']');
        Ok(())
    }
}

impl ser::SerializeTupleStruct for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b']');
        Ok(())
    }
}

impl ser::SerializeTupleVariant for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b']');
        Ok(())
    }
}

impl ser::SerializeMap for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(&mut **self)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b'}');
        Ok(())
    }
}

impl ser::SerializeStruct for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.fold_str(key);
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b'}');
        Ok(())
    }
}

impl ser::SerializeStructVariant for &mut ChecksumFolder {
    type Ok = ();
    type Error = ChecksumError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.fold_str(key);
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), Self::Error> {
        self.byte(b'}');
        Ok(())
    }
}
