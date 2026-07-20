//! Wire-compatible port of [schollz/pake/v3](https://github.com/schollz/pake) —
//! the password-authenticated key exchange (SPAKE2-style, Boneh–Shoup fig. 21)
//! used both between croc peers and between clients and the relay.
//!
//! Compatibility notes:
//! * Messages are JSON with Go's field names, including the Unicode
//!   subscripts (`Uᵤ`, `Xᵥ`, …); coordinates are arbitrary-precision decimal
//!   JSON numbers, exactly as Go's `big.Int` marshals.
//! * Curves: `p256`, `p384`, `p521` and `siec` (the nonstandard 255-bit
//!   "super-isolated" curve croc uses for the relay handshake, y² = x³ + 19).
//!   Go's `ed25519` option is not yet ported.
//! * Like the Go original (which uses `math/big`), this implementation is
//!   **not constant-time**. Migrating the standard curves to RustCrypto's
//!   constant-time arithmetic is tracked in MIGRATION.md.

use num_bigint::BigUint;
use num_traits::Zero;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum PakeError {
    UnknownCurve(String),
    BadMessage(String),
    NotOnCurve(&'static str),
    SameRole,
    NoSessionKey,
}

impl std::fmt::Display for PakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PakeError::UnknownCurve(c) => write!(f, "no such curve: {c}"),
            PakeError::BadMessage(m) => write!(f, "bad pake message: {m}"),
            PakeError::NotOnCurve(w) => write!(f, "{w} values not on curve"),
            PakeError::SameRole => write!(f, "can't have its own role"),
            PakeError::NoSessionKey => write!(f, "session key not generated"),
        }
    }
}

impl std::error::Error for PakeError {}

/// Affine point; `None` is the point at infinity.
type Point = Option<(BigUint, BigUint)>;

struct Curve {
    kind: CurveKind,
    p: BigUint,
    a: BigUint, // curve coefficient a, already reduced mod p
    b: BigUint,
    gx: BigUint,
    gy: BigUint,
}

fn hexu(s: &str) -> BigUint {
    BigUint::parse_bytes(s.as_bytes(), 16).expect("bad hex constant")
}

fn decu(s: &str) -> BigUint {
    BigUint::parse_bytes(s.as_bytes(), 10).expect("bad decimal constant")
}

/// Which arithmetic backend a curve uses.
#[derive(Clone, Copy, PartialEq)]
enum CurveKind {
    /// Constant-time RustCrypto backend (see [`ct`]).
    Std(ct::StdId),
    /// Variable-time `num-bigint` backend — the only option for SIEC, whose
    /// nonstandard curve has no constant-time implementation anywhere (Go's
    /// `tscholl2/siec` is variable-time too).
    Siec,
}

/// Constant-time scalar multiplication and point addition for the standard
/// NIST curves, backed by the audited RustCrypto `p256`/`p384`/`p521` crates.
///
/// The PAKE wire format is raw affine coordinates as big integers, so each
/// operation converts `num-bigint` coordinates in, runs the constant-time
/// group operation, and converts the affine result back out. Scalars are
/// reduced mod the group order first: Go's `crypto/elliptic.ScalarMult`
/// treats the scalar as an unreduced big-endian integer, but every one of
/// these curves has prime order and cofactor 1, so `k·P == (k mod n)·P` —
/// the point is identical, and RustCrypto requires a canonical scalar.
mod ct {
    use num_bigint::BigUint;

    pub type Point = Option<(BigUint, BigUint)>;

    #[derive(Clone, Copy, PartialEq)]
    pub enum StdId {
        P256,
        P384,
        P521,
    }

    macro_rules! std_backend {
        ($modname:ident, $krate:ident, $order_hex:literal) => {
            mod $modname {
                use super::Point;
                use num_bigint::BigUint;
                use $krate::elliptic_curve::group::Group as _;
                use $krate::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
                use $krate::elliptic_curve::PrimeField;
                use $krate::{AffinePoint, EncodedPoint, FieldBytes, ProjectivePoint, Scalar};

                fn order() -> BigUint {
                    BigUint::parse_bytes($order_hex, 16).unwrap()
                }

                /// Left-pad big-endian bytes to the curve's field width.
                fn pad(b: &[u8]) -> FieldBytes {
                    let mut out = FieldBytes::default();
                    let n = out.len();
                    let src = if b.len() > n { &b[b.len() - n..] } else { b };
                    out[n - src.len()..].copy_from_slice(src);
                    out
                }

                /// Reduce an arbitrary big-endian scalar mod n into a canonical
                /// curve scalar.
                fn scalar_from(k: &[u8]) -> Scalar {
                    let reduced = BigUint::from_bytes_be(k) % order();
                    let opt = Scalar::from_repr(pad(&reduced.to_bytes_be()));
                    Option::<Scalar>::from(opt).expect("reduced scalar is < n")
                }

                fn to_proj(pt: &Point) -> ProjectivePoint {
                    match pt {
                        None => ProjectivePoint::IDENTITY,
                        Some((x, y)) => {
                            let ep = EncodedPoint::from_affine_coordinates(
                                &pad(&x.to_bytes_be()),
                                &pad(&y.to_bytes_be()),
                                false,
                            );
                            Option::<AffinePoint>::from(AffinePoint::from_encoded_point(&ep))
                                .map(ProjectivePoint::from)
                                // Guarded upstream by is_on_curve; an off-curve
                                // point here would already have been rejected.
                                .unwrap_or(ProjectivePoint::IDENTITY)
                        }
                    }
                }

                fn from_proj(p: ProjectivePoint) -> Point {
                    let ep = p.to_affine().to_encoded_point(false);
                    match (ep.x(), ep.y()) {
                        (Some(x), Some(y)) => {
                            Some((BigUint::from_bytes_be(x), BigUint::from_bytes_be(y)))
                        }
                        _ => None, // identity
                    }
                }

                pub fn scalar_mult(x: &BigUint, y: &BigUint, k: &[u8]) -> Point {
                    from_proj(to_proj(&Some((x.clone(), y.clone()))) * scalar_from(k))
                }

                pub fn scalar_base_mult(k: &[u8]) -> Point {
                    from_proj(ProjectivePoint::generator() * scalar_from(k))
                }

                pub fn add(a: &Point, b: &Point) -> Point {
                    from_proj(to_proj(a) + to_proj(b))
                }
            }
        };
    }

    std_backend!(
        be256,
        p256,
        b"ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551"
    );
    std_backend!(
        be384,
        p384,
        b"ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973"
    );
    std_backend!(
        be521,
        p521,
        b"01fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffa51868783bf2f966b7fcc0148f709a5d03bb5c9b8899c47aebb6fb71e91386409"
    );

    pub fn scalar_mult(id: StdId, x: &BigUint, y: &BigUint, k: &[u8]) -> Point {
        match id {
            StdId::P256 => be256::scalar_mult(x, y, k),
            StdId::P384 => be384::scalar_mult(x, y, k),
            StdId::P521 => be521::scalar_mult(x, y, k),
        }
    }

    pub fn scalar_base_mult(id: StdId, k: &[u8]) -> Point {
        match id {
            StdId::P256 => be256::scalar_base_mult(k),
            StdId::P384 => be384::scalar_base_mult(k),
            StdId::P521 => be521::scalar_base_mult(k),
        }
    }

    pub fn add(id: StdId, a: &Point, b: &Point) -> Point {
        match id {
            StdId::P256 => be256::add(a, b),
            StdId::P384 => be384::add(a, b),
            StdId::P521 => be521::add(a, b),
        }
    }
}

/// Curve names supported by this port (Go additionally offers "ed25519").
pub fn available_curves() -> &'static [&'static str] {
    &["p256", "p384", "p521", "siec"]
}

impl Curve {
    fn by_name(name: &str) -> Result<Curve, PakeError> {
        // NIST parameters are the standard SEC2 values, matching Go's
        // crypto/elliptic; siec matches github.com/tscholl2/siec.
        match name {
            "p256" => {
                let p = hexu("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff");
                Ok(Curve {
                    kind: CurveKind::Std(ct::StdId::P256),
                    a: &p - 3u32,
                    b: hexu("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b"),
                    gx: hexu("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"),
                    gy: hexu("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"),
                    p,
                })
            }
            "p384" => {
                let p = hexu(
                    "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
                );
                Ok(Curve {
                    kind: CurveKind::Std(ct::StdId::P384),
                    a: &p - 3u32,
                    b: hexu("b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef"),
                    gx: hexu("aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7"),
                    gy: hexu("3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f"),
                    p,
                })
            }
            "p521" => {
                let p = hexu(
                    "01ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                );
                Ok(Curve {
                    kind: CurveKind::Std(ct::StdId::P521),
                    a: &p - 3u32,
                    b: hexu("0051953eb9618e1c9a1f929a21a0b68540eea2da725b99b315f3b8b489918ef109e156193951ec7e937b1652c0bd3bb1bf073573df883d2c34f1ef451fd46b503f00"),
                    gx: hexu("00c6858e06b70404e9cd9e3ecb662395b4429c648139053fb521f828af606b4d3dbaa14b5e77efe75928fe1dc127a2ffa8de3348b3c1856a429bf97e7e31c2e5bd66"),
                    gy: hexu("011839296a789a3bc0045c8a5fb42c7d1bd998f54449579b446817afbd17273e662c97ee72995ef42640c550b9013fad0761353c7086a272c24088be94769fd16650"),
                    p,
                })
            }
            "siec" => Ok(Curve {
                kind: CurveKind::Siec,
                p: decu(
                    "28948022309329048855892746252183396360603931420023084536990047309120118726721",
                ),
                a: BigUint::zero(),
                b: decu("19"),
                gx: decu("5"),
                gy: decu("12"),
            }),
            other => Err(PakeError::UnknownCurve(other.to_string())),
        }
    }

    fn is_on_curve(&self, pt: &Point) -> bool {
        match pt {
            None => false,
            Some((x, y)) => {
                let lhs = y.modpow(&BigUint::from(2u32), &self.p);
                let rhs = (x.modpow(&BigUint::from(3u32), &self.p)
                    + (&self.a * x) % &self.p
                    + &self.b)
                    % &self.p;
                lhs == rhs
            }
        }
    }

    fn sub_mod(&self, a: &BigUint, b: &BigUint) -> BigUint {
        ((a % &self.p) + &self.p - (b % &self.p)) % &self.p
    }

    fn inv_mod(&self, a: &BigUint) -> BigUint {
        // p is prime, so a^(p-2) is the inverse.
        a.modpow(&(&self.p - 2u32), &self.p)
    }

    fn neg_y(&self, pt: &Point) -> Point {
        pt.as_ref()
            .map(|(x, y)| (x.clone(), self.sub_mod(&BigUint::zero(), y)))
    }

    fn double(&self, pt: &Point) -> Point {
        let (x, y) = pt.as_ref()?;
        if y.is_zero() {
            return None;
        }
        let three_x2 = (BigUint::from(3u32) * x * x) % &self.p;
        let num = (three_x2 + &self.a) % &self.p;
        let den = self.inv_mod(&((BigUint::from(2u32) * y) % &self.p));
        let lambda = (num * den) % &self.p;
        let x3 = self.sub_mod(&((&lambda * &lambda) % &self.p), &((x + x) % &self.p));
        let y3 = self.sub_mod(&((&lambda * self.sub_mod(x, &x3)) % &self.p), y);
        Some((x3, y3))
    }

    fn add(&self, p1: &Point, p2: &Point) -> Point {
        if let CurveKind::Std(id) = self.kind {
            return ct::add(id, p1, p2);
        }
        // SIEC: variable-time affine addition over num-bigint.
        let (x1, y1) = match p1 {
            None => return p2.clone(),
            Some(v) => v,
        };
        let (x2, y2) = match p2 {
            None => return p1.clone(),
            Some(v) => v,
        };
        if x1 == x2 {
            if (y1 + y2) % &self.p == BigUint::zero() {
                return None;
            }
            return self.double(p1);
        }
        let lambda = (self.sub_mod(y2, y1) * self.inv_mod(&self.sub_mod(x2, x1))) % &self.p;
        let x3 = self.sub_mod(
            &self.sub_mod(&((&lambda * &lambda) % &self.p), x1),
            x2,
        );
        let y3 = self.sub_mod(&((&lambda * self.sub_mod(x1, &x3)) % &self.p), y1);
        Some((x3, y3))
    }

    /// Scalar multiplication. Standard NIST curves use the constant-time
    /// RustCrypto backend; SIEC uses double-and-add over the big-endian scalar
    /// bytes, matching Go's `crypto/elliptic` semantics (prime order, so
    /// reduction differences cannot change the result).
    fn scalar_mult(&self, pt: &Point, k: &[u8]) -> Point {
        if let CurveKind::Std(id) = self.kind {
            return match pt {
                None => None,
                Some((x, y)) => ct::scalar_mult(id, x, y, k),
            };
        }
        let mut result: Point = None;
        for byte in k {
            for bit in (0..8).rev() {
                result = self.double(&result);
                if (byte >> bit) & 1 == 1 {
                    result = self.add(&result, pt);
                }
            }
        }
        result
    }

    fn scalar_base_mult(&self, k: &[u8]) -> Point {
        if let CurveKind::Std(id) = self.kind {
            return ct::scalar_base_mult(id, k);
        }
        self.scalar_mult(&Some((self.gx.clone(), self.gy.clone())), k)
    }
}

/// The fixed "nothing-up-my-sleeve" U/V points from schollz/pake's
/// `initCurve`, which serve as the password-blinding generators.
fn uv_points(curve: &str) -> (BigUint, BigUint, BigUint, BigUint) {
    let (ux, uy, vx, vy) = match curve {
        "p256" => (
            "793136080485469241208656611513609866400481671852",
            "59748757929350367369315811184980635230185250460108398961713395032485227207304",
            "1086685267857089638167386722555472967068468061489",
            "9157340230202296554417312816309453883742349874205386245733062928888341584123",
        ),
        "p384" => (
            "793136080485469241208656611513609866400481671852",
            "7854890799382392388170852325516804266858248936799429260403044177981810983054351714387874260245230531084533936948596",
            "1086685267857089638167386722555472967068468061489",
            "21898206562669911998235297167979083576432197282633635629145270958059347586763418294901448537278960988843108277491616",
        ),
        "p521" => (
            "793136080485469241208656611513609866400481671852",
            "4032821203812196944795502391345776760852202059010382256134592838722123385325802540879231526503456158741518531456199762365161310489884151533417829496019094620",
            "1086685267857089638167386722555472967068468061489",
            "5010916268086655347194655708160715195931018676225831839835602465999566066450501167246678404591906342753230577187831311039273858772817427392089150297708931207",
        ),
        "siec" => (
            "793136080485469241208656611513609866400481671853",
            "18458907634222644275952014841865282643645472623913459400556233196838128612339",
            "1086685267857089638167386722555472967068468061489",
            "19593504966619549205903364028255899745298716108914514072669075231742699650911",
        ),
        _ => unreachable!("curve validated in by_name"),
    };
    (decu(ux), decu(uy), decu(vx), decu(vy))
}

/// Mirrors `pake.Pake`. Role 0 initiates (croc sender / relay client),
/// role 1 responds (croc recipient / relay server).
pub struct Pake {
    pub role: u8,
    curve: Curve,
    pw: Vec<u8>,
    u: Point,
    v: Point,
    x: Point,
    y: Point,
    vpw: Point,
    upw: Point,
    alpha: [u8; 32],
    k: Option<Vec<u8>>,
}

impl Drop for Pake {
    fn drop(&mut self) {
        // Wipe secret material: the weak password, the random blinding scalar,
        // and the derived session key. (Public curve points aren't secret.)
        use zeroize::Zeroize;
        self.pw.zeroize();
        self.alpha.zeroize();
        if let Some(k) = self.k.as_mut() {
            k.zeroize();
        }
    }
}

/// Minimal big-endian bytes, matching Go's `big.Int.Bytes()` (empty for 0).
fn go_bytes(n: &BigUint) -> Vec<u8> {
    if n.is_zero() {
        Vec::new()
    } else {
        n.to_bytes_be()
    }
}

fn coord_json(pt: &Point, idx: usize) -> String {
    match pt {
        Some(p) => {
            let v = if idx == 0 { &p.0 } else { &p.1 };
            v.to_str_radix(10)
        }
        None => "null".to_string(),
    }
}

impl Pake {
    /// Mirrors `pake.InitCurve(pw, role, curve)`.
    pub fn init_curve(pw: &[u8], role: u8, curve_name: &str) -> Result<Pake, PakeError> {
        let curve = Curve::by_name(curve_name)?;
        let (ux, uy, vx, vy) = uv_points(curve_name);
        let u: Point = Some((ux, uy));
        let v: Point = Some((vx, vy));
        if !curve.is_on_curve(&u) || !curve.is_on_curve(&v) {
            return Err(PakeError::NotOnCurve("U/V"));
        }
        let mut p = Pake {
            role: if role == 1 { 1 } else { 0 },
            curve,
            pw: pw.to_vec(),
            u,
            v,
            x: None,
            y: None,
            vpw: None,
            upw: None,
            alpha: [0u8; 32],
            k: None,
        };
        if p.role == 0 {
            // STEP: A computes X = U·pw + G·α
            p.vpw = p.curve.scalar_mult(&p.v, &p.pw);
            p.upw = p.curve.scalar_mult(&p.u, &p.pw);
            rand::thread_rng().fill_bytes(&mut p.alpha);
            let alpha_g = p.curve.scalar_base_mult(&p.alpha);
            p.x = p.curve.add(&p.upw, &alpha_g);
        }
        Ok(p)
    }

    /// JSON of the public variables, byte-format-compatible with Go's
    /// `Pake.Bytes()` (which marshals `Public()`).
    pub fn bytes(&self) -> Vec<u8> {
        format!(
            "{{\"Role\":{},\"Uᵤ\":{},\"Uᵥ\":{},\"Vᵤ\":{},\"Vᵥ\":{},\"Xᵤ\":{},\"Xᵥ\":{},\"Yᵤ\":{},\"Yᵥ\":{}}}",
            self.role,
            coord_json(&self.u, 0),
            coord_json(&self.u, 1),
            coord_json(&self.v, 0),
            coord_json(&self.v, 1),
            coord_json(&self.x, 0),
            coord_json(&self.x, 1),
            coord_json(&self.y, 0),
            coord_json(&self.y, 1),
        )
        .into_bytes()
    }

    /// Process the other party's `bytes()`. Mirrors `Pake.Update`.
    pub fn update(&mut self, q_bytes: &[u8]) -> Result<(), PakeError> {
        let v: serde_json::Value = serde_json::from_slice(q_bytes)
            .map_err(|e| PakeError::BadMessage(e.to_string()))?;
        let q_role = v
            .get("Role")
            .and_then(|r| r.as_u64())
            .ok_or_else(|| PakeError::BadMessage("missing Role".into()))?;
        if q_role == u64::from(self.role) {
            return Err(PakeError::SameRole);
        }

        let get_pt = |a: &str, b: &str| -> Result<Point, PakeError> {
            let read = |k: &str| -> Result<Option<BigUint>, PakeError> {
                match v.get(k) {
                    None | Some(serde_json::Value::Null) => Ok(None),
                    Some(serde_json::Value::Number(n)) => {
                        BigUint::parse_bytes(n.to_string().as_bytes(), 10)
                            .map(Some)
                            .ok_or_else(|| PakeError::BadMessage(format!("bad number in {k}")))
                    }
                    Some(_) => Err(PakeError::BadMessage(format!("bad type for {k}"))),
                }
            };
            match (read(a)?, read(b)?) {
                (Some(x), Some(y)) => Ok(Some((x, y))),
                _ => Ok(None),
            }
        };

        if self.role == 1 {
            // Received X from role 0; compute Y and the session key.
            let x = get_pt("Xᵤ", "Xᵥ")?;
            if !self.curve.is_on_curve(&x) {
                return Err(PakeError::NotOnCurve("X"));
            }
            self.x = x;
            self.vpw = self.curve.scalar_mult(&self.v, &self.pw);
            self.upw = self.curve.scalar_mult(&self.u, &self.pw);
            rand::thread_rng().fill_bytes(&mut self.alpha);
            let alpha_g = self.curve.scalar_base_mult(&self.alpha);
            self.y = self.curve.add(&self.vpw, &alpha_g);
            // Z = (X − U·pw)·α
            let z = self.curve.scalar_mult(
                &self.curve.add(&self.x, &self.curve.neg_y(&self.upw)),
                &self.alpha,
            );
            self.k = Some(self.session_hash(&z));
        } else {
            // Received Y from role 1; compute the session key.
            let y = get_pt("Yᵤ", "Yᵥ")?;
            if !self.curve.is_on_curve(&y) {
                return Err(PakeError::NotOnCurve("Y"));
            }
            self.y = y;
            // Z = (Y − V·pw)·α
            let z = self.curve.scalar_mult(
                &self.curve.add(&self.y, &self.curve.neg_y(&self.vpw)),
                &self.alpha,
            );
            self.k = Some(self.session_hash(&z));
        }
        Ok(())
    }

    /// K = SHA-256(pw ‖ X ‖ Y ‖ Z), coordinates as minimal big-endian bytes —
    /// identical to the Go hash transcript.
    fn session_hash(&self, z: &Point) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(&self.pw);
        for pt in [&self.x, &self.y, z] {
            if let Some((x, y)) = pt {
                h.update(go_bytes(x));
                h.update(go_bytes(y));
            }
        }
        h.finalize().to_vec()
    }

    /// Mirrors `Pake.SessionKey()`.
    pub fn session_key(&self) -> Result<Vec<u8>, PakeError> {
        self.k.clone().ok_or(PakeError::NoSessionKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: &str, y: &str) -> Point {
        Some((decu(x), decu(y)))
    }

    // Vectors generated from Go: crypto/elliptic and tscholl2/siec.
    // k = [1,2,3], pw = "some weak password".
    #[test]
    fn curve_math_matches_go() {
        let cases = [
            (
                "p256",
                "102177944418445466389220905852915392516949163071651313602197983576123586934735",
                "12754519112898262912616834608795559432138427757113692694370306044119752522562",
                "53954790233206707704377774197001360701189579251421636694579065471580808416597",
                "41929415471034965769699623633456831331823762153021418032864295447392317574423",
                "85261230923817659378034781912389283490647924559679374044289549934359765692098",
                "90372841657351955323498452116869639283295929300168252311841047427800105458529",
                "56580451076035890794735812961332685999617134615540236958551681516392327273195",
                "75288104128370351045713828736793861432249051783569416604062630277772205897021",
            ),
            (
                "siec",
                "20229788123255377252117609246816679794288126641783108573469038758141726144217",
                "24954944596292334650496455898477874666706713116404087873386411796931353338967",
                "7821604000828645370021527849864151781367895404633092807658104706533585607615",
                "12321748203907507311416299713967757533295516333623235765596715248801536832937",
                "12890957542364314192977455365464309255040581227397603771913365039752644473691",
                "1966715140277028004077810007564794649573112867679091036145080828506010855244",
                "18547106034652231084511426306268961189874505941601908436538648901407802923025",
                "8402302362712934719936415865028500915225426216290299538851902885977033418305",
            ),
        ];
        for (name, bx, by, px, py, ax, ay, dx, dy) in cases {
            let c = Curve::by_name(name).unwrap();
            let base = c.scalar_base_mult(&[1, 2, 3]);
            assert_eq!(base, pt(bx, by), "{name} scalar_base_mult");
            let scaled = c.scalar_mult(&base, b"some weak password");
            assert_eq!(scaled, pt(px, py), "{name} scalar_mult");
            assert_eq!(c.add(&base, &scaled), pt(ax, ay), "{name} add");
            assert_eq!(c.add(&base, &base), pt(dx, dy), "{name} double");
            assert!(c.is_on_curve(&base), "{name} on-curve");
        }
    }

    #[test]
    fn p384_p521_math_matches_go() {
        let cases = [
            (
                "p384",
                "24130927011173430378231057715197952681212665601905718919812090427010627717617469793155471216181827994232970479175536",
                "22037717132134218994409514148592082898077814907006468766203048987388822776133667992787817158513600959851507961684462",
                "37938645104099558529039088645282427725214109020491924994898352048444881929089432281343305530002613387212720251233369",
                "20411337777528479339208809955846131057199373993776380976847402659571748725478491323644617084795723963038548737212830",
            ),
            (
                "p521",
                "3661484409096487934920985446688295603408788383505517370011714675946370519279057366380235341438780144678641434360488908890221422137292668771035770549856068144",
                "5878886331948791724308241370702049630564158522048871648675704033410798172537062624939691818027757737380067027799817402093791327928224298943333530214666999536",
                "3479096684240030949358318169654864542756086130414806873765920570342335262632437335260474255738567934069355392222537586631894323327038117389362721239740982185",
                "2850673940125373004196833367292998411493512986424629351274559082111462554595354362522690709984291995078991396962260758567978184395564360311638298205431254799",
            ),
        ];
        for (name, bx, by, px, py) in cases {
            let c = Curve::by_name(name).unwrap();
            let base = c.scalar_base_mult(&[1, 2, 3]);
            assert_eq!(base, pt(bx, by), "{name} scalar_base_mult");
            let scaled = c.scalar_mult(&base, b"some weak password");
            assert_eq!(scaled, pt(px, py), "{name} scalar_mult");
        }
    }

    // The standard curves must use the constant-time RustCrypto backend and
    // SIEC the bignum backend; guards against a future refactor silently
    // routing a NIST curve back through variable-time code.
    #[test]
    fn backend_selection() {
        for name in ["p256", "p384", "p521"] {
            assert!(
                matches!(Curve::by_name(name).unwrap().kind, CurveKind::Std(_)),
                "{name} should use the constant-time backend"
            );
        }
        assert!(matches!(Curve::by_name("siec").unwrap().kind, CurveKind::Siec));
    }

    // A large scalar (top bit set, so ≥ n for p256) must still agree between
    // the constant-time backend and a direct reduction — exercising the
    // reduce-mod-n path the RustCrypto scalar requires.
    #[test]
    fn large_scalar_reduces_like_go() {
        let c = Curve::by_name("p256").unwrap();
        let k = [0xffu8; 32]; // 2^256 - 1 > n
        // k·G computed via the backend must be a valid on-curve point equal to
        // (k mod n)·G. Recompute (k mod n)·G by feeding the reduced bytes.
        let n = decu(
            "115792089210356248762697446949407573529996955224135760342422259061068512044369",
        );
        let reduced = (BigUint::from_bytes_be(&k) % &n).to_bytes_be();
        assert_eq!(
            c.scalar_base_mult(&k),
            c.scalar_base_mult(&reduced),
            "k·G must equal (k mod n)·G"
        );
        assert!(c.is_on_curve(&c.scalar_base_mult(&k)));
    }

    #[test]
    fn full_exchange_all_curves() {
        for curve in available_curves() {
            let mut a = Pake::init_curve(b"shared-secret", 0, curve).unwrap();
            let mut b = Pake::init_curve(b"shared-secret", 1, curve).unwrap();
            b.update(&a.bytes()).unwrap();
            a.update(&b.bytes()).unwrap();
            assert_eq!(
                a.session_key().unwrap(),
                b.session_key().unwrap(),
                "curve {curve}"
            );
            assert_eq!(a.session_key().unwrap().len(), 32);
        }
    }

    #[test]
    fn wrong_password_differs() {
        let mut a = Pake::init_curve(b"secret-one", 0, "siec").unwrap();
        let mut b = Pake::init_curve(b"secret-two", 1, "siec").unwrap();
        b.update(&a.bytes()).unwrap();
        a.update(&b.bytes()).unwrap();
        assert_ne!(a.session_key().unwrap(), b.session_key().unwrap());
    }

    #[test]
    fn rejects_same_role_and_bad_points() {
        let mut a = Pake::init_curve(b"x", 0, "p256").unwrap();
        let a2 = Pake::init_curve(b"x", 0, "p256").unwrap();
        assert!(matches!(a.update(&a2.bytes()), Err(PakeError::SameRole)));

        let mut b = Pake::init_curve(b"x", 1, "p256").unwrap();
        let forged = br#"{"Role":0,"Xu":1,"Xv":2}"#;
        // missing real X coordinates → not on curve
        assert!(b.update(forged).is_err());
    }

    #[test]
    fn json_shape_matches_go() {
        let a = Pake::init_curve(b"x", 1, "siec").unwrap();
        let s = String::from_utf8(a.bytes()).unwrap();
        // Role-1 before update: U/V set, X/Y null — exactly as Go's Public().
        assert!(s.contains("\"Role\":1"));
        assert!(s.contains("\"Uᵤ\":793136080485469241208656611513609866400481671853"));
        assert!(s.contains("\"Xᵤ\":null"));
    }
}
