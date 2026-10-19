//! A hand-written schema on `rusty_prost` and the same schema derived by `prost` (the oracle),
//! plus a seeded generator, so the two can be compared on the same values.
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;

use rusty_prost::{field, Field, Message, Result, Writer};

pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// A tiny deterministic generator, so a failure reproduces without a dependency.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n
    }
    pub fn chance(&mut self) -> bool {
        self.next() % 2 == 0
    }
}

// ---- ours ----

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Inner {
    pub id: i32,
    pub name: String,
}

impl Message for Inner {
    fn encode(&self, w: &mut Writer) {
        if self.id != 0 {
            w.int32(1, self.id);
        }
        if !self.name.is_empty() {
            w.string(2, &self.name);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.id = f.int32()?,
            2 => self.name = f.str()?.to_owned(),
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum One {
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
    pub m: BTreeMap<String, i32>,
    pub one: Option<One>,
    pub e: i32,
    pub pf32: Vec<u32>,
    pub pdb: Vec<f64>,
    pub ps64: Vec<i64>,
}

impl Message for All {
    fn encode(&self, w: &mut Writer) {
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
        for (k, v) in &self.m {
            w.message(20, |w| {
                w.string(1, k);
                w.int32(2, *v);
            });
        }
        match &self.one {
            Some(One::Int(v)) => w.int32(21, *v),
            Some(One::Str(v)) => w.string(22, v),
            None => {}
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
            w.packed_varints(26, self.ps64.iter().map(|&v| rusty_protobuf::zigzag64(v)));
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.i32_ = f.int32()?,
            2 => self.i64_ = f.int64()?,
            3 => self.u32_ = f.uint32()?,
            4 => self.u64_ = f.uint64()?,
            5 => self.s32 = f.sint32()?,
            6 => self.s64 = f.sint64()?,
            7 => self.b = f.bool()?,
            8 => self.f32_ = f.fixed32()?,
            9 => self.f64_ = f.fixed64()?,
            10 => self.sf32 = f.sfixed32()?,
            11 => self.sf64 = f.sfixed64()?,
            12 => self.fl = f.float()?,
            13 => self.db = f.double()?,
            14 => self.s = f.str()?.to_owned(),
            15 => self.by = f.bytes()?.to_vec(),
            16 => field::message(&mut self.inner, &f)?,
            17 => {
                for v in f.packed_varints()? {
                    self.packed.push(v? as i32);
                }
            }
            18 => self.strs.push(f.str()?.to_owned()),
            19 => field::push_message(&mut self.inners, &f)?,
            20 => field::map_entry(&mut self.m, &f, |k| Ok(k.str()?.to_owned()), |v| v.int32())?,
            21 => self.one = Some(One::Int(f.int32()?)),
            22 => self.one = Some(One::Str(f.str()?.to_owned())),
            23 => self.e = f.int32()?,
            24 => self.pf32.extend(f.packed_fixed32()?),
            25 => self.pdb.extend(f.packed_fixed64()?.map(f64::from_bits)),
            26 => {
                for v in f.packed_varints()? {
                    self.ps64.push(rusty_protobuf::unzigzag64(v?));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

// ---- the oracle: the same schema, derived by prost ----

#[derive(Clone, PartialEq, prost::Message)]
pub struct PInner {
    #[prost(int32, tag = "1")]
    pub id: i32,
    #[prost(string, tag = "2")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PAll {
    #[prost(int32, tag = "1")]
    pub i32_: i32,
    #[prost(int64, tag = "2")]
    pub i64_: i64,
    #[prost(uint32, tag = "3")]
    pub u32_: u32,
    #[prost(uint64, tag = "4")]
    pub u64_: u64,
    #[prost(sint32, tag = "5")]
    pub s32: i32,
    #[prost(sint64, tag = "6")]
    pub s64: i64,
    #[prost(bool, tag = "7")]
    pub b: bool,
    #[prost(fixed32, tag = "8")]
    pub f32_: u32,
    #[prost(fixed64, tag = "9")]
    pub f64_: u64,
    #[prost(sfixed32, tag = "10")]
    pub sf32: i32,
    #[prost(sfixed64, tag = "11")]
    pub sf64: i64,
    #[prost(float, tag = "12")]
    pub fl: f32,
    #[prost(double, tag = "13")]
    pub db: f64,
    #[prost(string, tag = "14")]
    pub s: String,
    #[prost(bytes = "vec", tag = "15")]
    pub by: Vec<u8>,
    #[prost(message, optional, tag = "16")]
    pub inner: Option<PInner>,
    #[prost(int32, repeated, tag = "17")]
    pub packed: Vec<i32>,
    #[prost(string, repeated, tag = "18")]
    pub strs: Vec<String>,
    #[prost(message, repeated, tag = "19")]
    pub inners: Vec<PInner>,
    #[prost(btree_map = "string, int32", tag = "20")]
    pub m: BTreeMap<String, i32>,
    #[prost(oneof = "pone::One", tags = "21, 22")]
    pub one: Option<pone::One>,
    #[prost(int32, tag = "23")]
    pub e: i32,
    #[prost(fixed32, repeated, tag = "24")]
    pub pf32: Vec<u32>,
    #[prost(double, repeated, tag = "25")]
    pub pdb: Vec<f64>,
    #[prost(sint64, repeated, tag = "26")]
    pub ps64: Vec<i64>,
}

pub mod pone {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum One {
        #[prost(int32, tag = "21")]
        Int(i32),
        #[prost(string, tag = "22")]
        Str(String),
    }
}

impl From<&All> for PAll {
    fn from(a: &All) -> PAll {
        PAll {
            i32_: a.i32_,
            i64_: a.i64_,
            u32_: a.u32_,
            u64_: a.u64_,
            s32: a.s32,
            s64: a.s64,
            b: a.b,
            f32_: a.f32_,
            f64_: a.f64_,
            sf32: a.sf32,
            sf64: a.sf64,
            fl: a.fl,
            db: a.db,
            s: a.s.clone(),
            by: a.by.clone(),
            inner: a.inner.as_ref().map(|i| PInner {
                id: i.id,
                name: i.name.clone(),
            }),
            packed: a.packed.clone(),
            strs: a.strs.clone(),
            inners: a
                .inners
                .iter()
                .map(|i| PInner {
                    id: i.id,
                    name: i.name.clone(),
                })
                .collect(),
            m: a.m.clone(),
            one: a.one.as_ref().map(|o| match o {
                One::Int(v) => pone::One::Int(*v),
                One::Str(v) => pone::One::Str(v.clone()),
            }),
            e: a.e,
            pf32: a.pf32.clone(),
            pdb: a.pdb.clone(),
            ps64: a.ps64.clone(),
        }
    }
}

// ---- generator ----

const WORDS: &[&str] = &["", "a", "h\u{e9}llo", "\u{2603}", "tools/call", "x"];

fn word(rng: &mut Lcg) -> String {
    if rng.below(12) == 0 {
        "y".repeat(120 + rng.below(20)) // straddles the 127/128-byte length boundary
    } else {
        WORDS[rng.below(WORDS.len())].to_owned()
    }
}

fn inner(rng: &mut Lcg) -> Inner {
    Inner {
        id: if rng.chance() { rng.next() as i32 } else { 0 },
        name: word(rng),
    }
}

/// A random message. Map keys are non-empty and values non-zero: the reference writes map
/// entries in full while `prost` drops default keys and values, which decodes the same but
/// differs in bytes.
pub fn random_all(rng: &mut Lcg) -> All {
    let mut a = All::default();
    if rng.chance() {
        a.i32_ = rng.next() as i32;
    }
    if rng.chance() {
        a.i64_ = (rng.next() as i64) << 20 ^ rng.next() as i64;
    }
    if rng.chance() {
        a.u32_ = rng.next() as u32;
    }
    if rng.chance() {
        a.u64_ = rng.next() << 31 | rng.next();
    }
    if rng.chance() {
        a.s32 = rng.next() as i32;
    }
    if rng.chance() {
        a.s64 = -(rng.next() as i64);
    }
    a.b = rng.chance();
    if rng.chance() {
        a.f32_ = rng.next() as u32;
        a.f64_ = rng.next() << 20;
        a.sf32 = -(rng.next() as i32);
        a.sf64 = rng.next() as i64;
    }
    if rng.chance() {
        a.fl = (rng.next() as i32) as f32 / 8.0;
        a.db = (rng.next() as i64) as f64 / 16.0;
    }
    a.s = word(rng);
    if rng.chance() {
        a.by = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
    }
    if rng.chance() {
        a.inner = Some(inner(rng));
    }
    a.packed = (0..rng.below(6)).map(|_| rng.next() as i32).collect();
    a.strs = (0..rng.below(4)).map(|_| word(rng)).collect();
    a.inners = (0..rng.below(4)).map(|_| inner(rng)).collect();
    for _ in 0..rng.below(4) {
        let key = format!("k{}", rng.below(50));
        a.m.insert(key, 1 + rng.below(1000) as i32);
    }
    a.one = match rng.below(3) {
        0 => None,
        1 => Some(One::Int(1 + rng.below(100) as i32)),
        _ => Some(One::Str(word(rng))),
    };
    if rng.chance() {
        a.e = rng.below(3) as i32;
    }
    a.pf32 = (0..rng.below(4)).map(|_| rng.next() as u32).collect();
    a.pdb = (0..rng.below(4))
        .map(|_| (rng.next() as i32) as f64 / 4.0)
        .collect();
    a.ps64 = (0..rng.below(4))
        .map(|_| rng.next() as i64 - (1 << 30))
        .collect();
    a
}
