use std::hash::Hasher;

use serde::ser::{self, Serialize};

use super::KeyHasher;

#[derive(Debug)]
pub(super) struct Unhashable;

impl std::fmt::Display for Unhashable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a value refused to serialize")
    }
}

impl std::error::Error for Unhashable {}

impl ser::Error for Unhashable {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Unhashable
    }
}

const ITEM: u8 = 0xf1;
const END: u8 = 0xf2;
const SKIPPED: u8 = 0xf4;
const FAILED: u8 = 0xf3;
const DONE: u8 = 0xff;

pub(super) fn hash_value(h: &mut KeyHasher, v: &(impl Serialize + ?Sized)) {
    if v.serialize(&mut *h).is_err() {
        h.write_u8(FAILED);
    }
    h.write_u8(DONE);
}

type Done = Result<(), Unhashable>;

impl ser::Serializer for &mut KeyHasher {
    type Ok = ();
    type Error = Unhashable;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Done {
        self.write_u8(u8::from(v));
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Done {
        self.write_i8(v);
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Done {
        self.write_i16(v);
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Done {
        self.write_i32(v);
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Done {
        self.write_i64(v);
        Ok(())
    }
    fn serialize_i128(self, v: i128) -> Done {
        self.write_i128(v);
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Done {
        self.write_u8(v);
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Done {
        self.write_u16(v);
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Done {
        self.write_u32(v);
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Done {
        self.write_u64(v);
        Ok(())
    }
    fn serialize_u128(self, v: u128) -> Done {
        self.write_u128(v);
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Done {
        self.write_u32(v.to_bits());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Done {
        self.write_u64(v.to_bits());
        Ok(())
    }
    fn serialize_char(self, v: char) -> Done {
        self.write_u32(u32::from(v));
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Done {
        self.write(v.as_bytes());
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Done {
        self.write(v);
        Ok(())
    }
    fn serialize_none(self) -> Done {
        self.write_u8(0);
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Done {
        self.write_u8(1);
        v.serialize(self)
    }
    fn serialize_unit(self) -> Done {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Done {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, index: u32, _: &'static str) -> Done {
        self.write_u32(index);
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> Done {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(self, _: &'static str, index: u32, _: &'static str, v: &T) -> Done {
        self.write_u32(index);
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Unhashable> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Unhashable> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Unhashable> {
        Ok(self)
    }
    fn serialize_tuple_variant(self, _: &'static str, index: u32, _: &'static str, _: usize) -> Result<Self, Unhashable> {
        self.write_u32(index);
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, Unhashable> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Unhashable> {
        Ok(self)
    }
    fn serialize_struct_variant(self, _: &'static str, index: u32, _: &'static str, _: usize) -> Result<Self, Unhashable> {
        self.write_u32(index);
        Ok(self)
    }
}

macro_rules! elements {
    ($($t:ident :: $f:ident),*) => {$(
        impl ser::$t for &mut KeyHasher {
            type Ok = ();
            type Error = Unhashable;
            fn $f<T: ?Sized + Serialize>(&mut self, v: &T) -> Done {
                self.write_u8(ITEM);
                v.serialize(&mut **self)
            }
            fn end(self) -> Done {
                self.write_u8(END);
                Ok(())
            }
        }
    )*};
}

elements!(SerializeSeq::serialize_element, SerializeTuple::serialize_element, SerializeTupleStruct::serialize_field, SerializeTupleVariant::serialize_field);

macro_rules! fields {
    ($($t:ident),*) => {$(
        impl ser::$t for &mut KeyHasher {
            type Ok = ();
            type Error = Unhashable;
            fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &'static str, v: &T) -> Done {
                self.write_u8(ITEM);
                v.serialize(&mut **self)
            }
            fn skip_field(&mut self, _: &'static str) -> Done {
                self.write_u8(SKIPPED);
                Ok(())
            }
            fn end(self) -> Done {
                self.write_u8(END);
                Ok(())
            }
        }
    )*};
}

fields!(SerializeStruct, SerializeStructVariant);

impl ser::SerializeMap for &mut KeyHasher {
    type Ok = ();
    type Error = Unhashable;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, k: &T) -> Done {
        self.write_u8(ITEM);
        k.serialize(&mut **self)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Done {
        v.serialize(&mut **self)
    }
    fn end(self) -> Done {
        self.write_u8(END);
        Ok(())
    }
}
