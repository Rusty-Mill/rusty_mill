//! A hand-written schema over the runtime, the one `fixtures/gen.py` documents: what a user of
//! the crate writes by hand. Decoders skip unknown fields, last scalar wins, sub-messages merge,
//! repeated fields concatenate.
#![allow(dead_code, clippy::unwrap_used)]

use rusty_protobuf::{unzigzag64, zigzag64, Error, Reader, Writer};

pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Inner {
    pub id: i32,
    pub name: String,
}

impl Inner {
    pub fn decode(r: Reader<'_>) -> Result<Inner, Error> {
        let mut m = Inner::default();
        m.merge(r)?;
        Ok(m)
    }

    pub fn merge(&mut self, r: Reader<'_>) -> Result<(), Error> {
        for f in r {
            let f = f?;
            match f.number {
                1 => self.id = f.int32()?,
                2 => self.name = f.str()?.to_owned(),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut Writer) {
        if self.id != 0 {
            w.int32(1, self.id);
        }
        if !self.name.is_empty() {
            w.string(2, &self.name);
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum Oneof {
    #[default]
    None,
    Int(i32),
    Str(String),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct All {
    pub i32_: i32,
    pub i64_: i64,
    pub u32_: u32,
    pub u64_: u64,
    pub s32: i32,
    pub s64: i64,
    pub b: bool,
    pub f32_: u32,
    pub f64_: u64,
    pub sf32: i32,
    pub sf64: i64,
    pub fl: f32,
    pub db: f64,
    pub s: String,
    pub by: Vec<u8>,
    pub inner: Option<Inner>,
    pub packed: Vec<i32>,
    pub strs: Vec<String>,
    pub inners: Vec<Inner>,
    pub map: Vec<(String, i32)>,
    pub one: Oneof,
    pub e: i32,
    pub pf32: Vec<u32>,
    pub pdb: Vec<f64>,
    pub ps64: Vec<i64>,
}

fn map_entry(r: Reader<'_>) -> Result<(String, i32), Error> {
    let (mut k, mut v) = (String::new(), 0);
    for f in r {
        let f = f?;
        match f.number {
            1 => k = f.str()?.to_owned(),
            2 => v = f.int32()?,
            _ => {}
        }
    }
    Ok((k, v))
}

impl All {
    pub fn decode(buf: &[u8]) -> Result<All, Error> {
        let mut m = All::default();
        for f in Reader::new(buf) {
            let f = f?;
            match f.number {
                1 => m.i32_ = f.int32()?,
                2 => m.i64_ = f.int64()?,
                3 => m.u32_ = f.uint32()?,
                4 => m.u64_ = f.uint64()?,
                5 => m.s32 = f.sint32()?,
                6 => m.s64 = f.sint64()?,
                7 => m.b = f.bool()?,
                8 => m.f32_ = f.fixed32()?,
                9 => m.f64_ = f.fixed64()?,
                10 => m.sf32 = f.sfixed32()?,
                11 => m.sf64 = f.sfixed64()?,
                12 => m.fl = f.float()?,
                13 => m.db = f.double()?,
                14 => m.s = f.str()?.to_owned(),
                15 => m.by = f.bytes()?.to_vec(),
                16 => m
                    .inner
                    .get_or_insert_with(Inner::default)
                    .merge(f.message()?)?,
                17 => {
                    for v in f.packed_varints()? {
                        m.packed.push(v? as i32);
                    }
                }
                18 => m.strs.push(f.str()?.to_owned()),
                19 => m.inners.push(Inner::decode(f.message()?)?),
                20 => m.map.push(map_entry(f.message()?)?),
                21 => m.one = Oneof::Int(f.int32()?),
                22 => m.one = Oneof::Str(f.str()?.to_owned()),
                23 => m.e = f.int32()?,
                24 => m.pf32.extend(f.packed_fixed32()?),
                25 => m.pdb.extend(f.packed_fixed64()?.map(f64::from_bits)),
                26 => {
                    for v in f.packed_varints()? {
                        m.ps64.push(unzigzag64(v?));
                    }
                }
                _ => {}
            }
        }
        Ok(m)
    }

    /// Fields in number order, proto3 defaults omitted: what the reference serialises.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::new();
        if self.i32_ != 0 {
            w.int32(1, self.i32_);
        }
        if self.i64_ != 0 {
            w.int64(2, self.i64_);
        }
        if self.u32_ != 0 {
            w.uint32(3, self.u32_);
        }
        if self.u64_ != 0 {
            w.uint64(4, self.u64_);
        }
        if self.s32 != 0 {
            w.sint32(5, self.s32);
        }
        if self.s64 != 0 {
            w.sint64(6, self.s64);
        }
        if self.b {
            w.bool(7, true);
        }
        if self.f32_ != 0 {
            w.fixed32(8, self.f32_);
        }
        if self.f64_ != 0 {
            w.fixed64(9, self.f64_);
        }
        if self.sf32 != 0 {
            w.sfixed32(10, self.sf32);
        }
        if self.sf64 != 0 {
            w.sfixed64(11, self.sf64);
        }
        if self.fl != 0.0 {
            w.float(12, self.fl);
        }
        if self.db != 0.0 {
            w.double(13, self.db);
        }
        if !self.s.is_empty() {
            w.string(14, &self.s);
        }
        if !self.by.is_empty() {
            w.bytes(15, &self.by);
        }
        if let Some(inner) = &self.inner {
            w.message(16, |w| inner.encode(w));
        }
        if !self.packed.is_empty() {
            w.packed_varints(17, self.packed.iter().map(|&v| i64::from(v) as u64));
        }
        for s in &self.strs {
            w.string(18, s);
        }
        for i in &self.inners {
            w.message(19, |w| i.encode(w));
        }
        for (k, v) in &self.map {
            w.message(20, |w| {
                w.string(1, k);
                w.int32(2, *v);
            });
        }
        match &self.one {
            Oneof::None => {}
            Oneof::Int(v) => w.int32(21, *v),
            Oneof::Str(v) => w.string(22, v),
        }
        if self.e != 0 {
            w.int32(23, self.e);
        }
        if !self.pf32.is_empty() {
            w.packed_fixed32(24, self.pf32.iter().copied());
        }
        if !self.pdb.is_empty() {
            w.packed_fixed64(25, self.pdb.iter().map(|v| v.to_bits()));
        }
        if !self.ps64.is_empty() {
            w.packed_varints(26, self.ps64.iter().map(|&v| zigzag64(v)));
        }
        w.finish()
    }
}
