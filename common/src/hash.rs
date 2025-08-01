use malachite_bigint::BigInt;
use num_traits::ToPrimitive;
use siphasher::sip::SipHasher24;
use std::hash::{BuildHasher, Hash, Hasher};

pub type PyHash = i64;
pub type PyUHash = u64;

/// A PyHash value used to represent a missing hash value, e.g. means "not yet computed" for
/// `str`'s hash cache
pub const SENTINEL: PyHash = -1;

/// Prime multiplier used in string and various other hashes.
pub const MULTIPLIER: PyHash = 1_000_003; // 0xf4243
/// Numeric hashes are based on reduction modulo the prime 2**_BITS - 1
pub const BITS: usize = 61;
pub const MODULUS: PyUHash = (1 << BITS) - 1;
pub const INF: PyHash = 314_159;
pub const NAN: PyHash = 0;
pub const IMAG: PyHash = MULTIPLIER;
pub const ALGO: &str = "siphash24";
pub const HASH_BITS: usize = std::mem::size_of::<PyHash>() * 8;
// SipHasher24 takes 2 u64s as a seed
pub const SEED_BITS: usize = std::mem::size_of::<u64>() * 2 * 8;

// pub const CUTOFF: usize = 7;

pub struct HashSecret {
    k0: u64,
    k1: u64,
}

impl BuildHasher for HashSecret {
    type Hasher = SipHasher24;

    fn build_hasher(&self) -> Self::Hasher {
        SipHasher24::new_with_keys(self.k0, self.k1)
    }
}

impl HashSecret {
    pub fn new(seed: u32) -> Self {
        let mut buf = [0u8; 16];
        lcg_urandom(seed, &mut buf);
        let (left, right) = buf.split_at(8);
        let k0 = u64::from_le_bytes(left.try_into().unwrap());
        let k1 = u64::from_le_bytes(right.try_into().unwrap());
        Self { k0, k1 }
    }
}

impl HashSecret {
    pub fn hash_value<T: Hash + ?Sized>(&self, data: &T) -> PyHash {
        fix_sentinel(mod_int(self.hash_one(data) as _))
    }

    pub fn hash_iter<'a, T: 'a, I, F, E>(&self, iter: I, hash_func: F) -> Result<PyHash, E>
    where
        I: IntoIterator<Item = &'a T>,
        F: Fn(&'a T) -> Result<PyHash, E>,
    {
        let mut hasher = self.build_hasher();
        for element in iter {
            let item_hash = hash_func(element)?;
            item_hash.hash(&mut hasher);
        }
        Ok(fix_sentinel(mod_int(hasher.finish() as PyHash)))
    }

    pub fn hash_bytes(&self, value: &[u8]) -> PyHash {
        if value.is_empty() {
            0
        } else {
            self.hash_value(value)
        }
    }

    pub fn hash_str(&self, value: &str) -> PyHash {
        self.hash_bytes(value.as_bytes())
    }
}

#[inline]
pub const fn hash_pointer(value: usize) -> PyHash {
    // TODO: 32bit?
    let hash = (value >> 4) | value;
    hash as _
}

#[inline]
pub fn hash_float(value: f64) -> Option<PyHash> {
    // cpython _Py_HashDouble
    if !value.is_finite() {
        return if value.is_infinite() {
            Some(if value > 0.0 { INF } else { -INF })
        } else {
            None
        };
    }

    let frexp = super::float_ops::decompose_float(value);

    // process 28 bits at a time;  this should work well both for binary
    // and hexadecimal floating point.
    let mut m = frexp.0;
    let mut e = frexp.1;
    let mut x: PyUHash = 0;
    while m != 0.0 {
        x = ((x << 28) & MODULUS) | (x >> (BITS - 28));
        m *= 268_435_456.0; // 2**28
        e -= 28;
        let y = m as PyUHash; // pull out integer part
        m -= y as f64;
        x += y;
        if x >= MODULUS {
            x -= MODULUS;
        }
    }

    // adjust for the exponent;  first reduce it modulo BITS
    const BITS32: i32 = BITS as i32;
    e = if e >= 0 {
        e % BITS32
    } else {
        BITS32 - 1 - ((-1 - e) % BITS32)
    };
    x = ((x << e) & MODULUS) | (x >> (BITS32 - e));

    Some(fix_sentinel(x as PyHash * value.signum() as PyHash))
}

pub fn hash_bigint(value: &BigInt) -> PyHash {
    let ret = match value.to_i64() {
        Some(i) => mod_int(i),
        None => (value % MODULUS).to_i64().unwrap_or_else(|| unsafe {
            // SAFETY: MODULUS < i64::MAX, so value % MODULUS is guaranteed to be in the range of i64
            std::hint::unreachable_unchecked()
        }),
    };
    fix_sentinel(ret)
}

#[inline]
pub const fn hash_usize(data: usize) -> PyHash {
    fix_sentinel(mod_int(data as i64))
}

#[inline(always)]
pub const fn fix_sentinel(x: PyHash) -> PyHash {
    if x == SENTINEL { -2 } else { x }
}

#[inline]
pub const fn mod_int(value: i64) -> PyHash {
    value % MODULUS as i64
}

pub fn lcg_urandom(mut x: u32, buf: &mut [u8]) {
    for b in buf {
        x = x.wrapping_mul(214013);
        x = x.wrapping_add(2531011);
        *b = ((x >> 16) & 0xff) as u8;
    }
}

#[inline]
pub const fn hash_object_id_raw(p: usize) -> PyHash {
    // TODO: Use commented logic when below issue resolved.
    // Ref: https://github.com/RustPython/RustPython/pull/3951#issuecomment-1193108966

    /* bottom 3 or 4 bits are likely to be 0; rotate y by 4 to avoid
    excessive hash collisions for dicts and sets */
    // p.rotate_right(4) as PyHash
    p as PyHash
}

#[inline]
pub const fn hash_object_id(p: usize) -> PyHash {
    fix_sentinel(hash_object_id_raw(p))
}

pub fn keyed_hash(key: u64, buf: &[u8]) -> u64 {
    let mut hasher = SipHasher24::new_with_keys(key, 0);
    buf.hash(&mut hasher);
    hasher.finish()
}
