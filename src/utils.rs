//! Port of the parts of `src/utils` needed so far.
//!
//! The Go package is a grab-bag (hashing, filesystem walking, IP discovery,
//! progress helpers); pieces are ported as the modules that need them arrive.

use crate::mnemonicode;
use md5::{Digest, Md5};
use rand::Rng;
use std::io::Read;
use std::path::Path;

/// Matches Go's `utils.NbPinNumbers`.
pub const NB_PIN_NUMBERS: usize = 4;
/// Matches Go's `utils.NbBytesWords`.
pub const NB_BYTES_WORDS: usize = 4;

/// Random numeric pin, mirroring `utils.GenerateRandomPin`
/// (Go draws each digit from `[0, 9)`).
pub fn generate_random_pin() -> String {
    let mut rng = rand::thread_rng();
    (0..NB_PIN_NUMBERS)
        .map(|_| char::from(b'0' + rng.gen_range(0..9u8)))
        .collect()
}

/// Random code phrase like `1234-quiet-lion-daisy`, mirroring
/// `utils.GetRandomName`.
pub fn get_random_name() -> String {
    let mut bs = [0u8; NB_BYTES_WORDS];
    rand::thread_rng().fill(&mut bs);
    let words = mnemonicode::encode_word_list(&bs);
    format!("{}-{}", generate_random_pin(), words.join("-"))
}

/// imohash with croc's "partial" parameters (`utils.IMOHashFile`):
/// sample the first/middle/last 2 MiB for files ≥ 8 MiB, murmur3-x64-128,
/// then overwrite the first bytes with the uvarint-encoded file size.
pub fn imohash_file(path: &Path) -> std::io::Result<Vec<u8>> {
    const SAMPLE_SIZE: u64 = 16 * 16 * 8 * 1024; // 2 MiB, croc's imopartial
    const SAMPLE_THRESHOLD: u64 = 128 * 1024;

    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut data = Vec::new();
    if size < SAMPLE_THRESHOLD || size < 4 * SAMPLE_SIZE {
        f.read_to_end(&mut data)?;
    } else {
        use std::io::{Seek, SeekFrom};
        let mut buf = vec![0u8; SAMPLE_SIZE as usize];
        f.read_exact(&mut buf)?;
        data.extend_from_slice(&buf);
        f.seek(SeekFrom::Start(size / 2))?;
        f.read_exact(&mut buf)?;
        data.extend_from_slice(&buf);
        f.seek(SeekFrom::End(-(SAMPLE_SIZE as i64)))?;
        f.read_exact(&mut buf)?;
        data.extend_from_slice(&buf);
    }
    let h = murmur3::murmur3_x64_128(&mut std::io::Cursor::new(&data), 0)
        .map_err(std::io::Error::other)?;
    // twmb/murmur3's Sum lays out h1 then h2 as big-endian.
    let h1 = (h & 0xffff_ffff_ffff_ffff) as u64;
    let h2 = (h >> 64) as u64;
    let mut hash = [0u8; 16];
    hash[..8].copy_from_slice(&h1.to_be_bytes());
    hash[8..].copy_from_slice(&h2.to_be_bytes());
    put_uvarint(&mut hash, size);
    Ok(hash.to_vec())
}

/// Go's `binary.PutUvarint` into the head of `buf`.
fn put_uvarint(buf: &mut [u8], mut x: u64) {
    let mut i = 0;
    while x >= 0x80 {
        buf[i] = (x as u8) | 0x80;
        x >>= 7;
        i += 1;
    }
    buf[i] = x as u8;
}

/// HighwayHash-256 with croc's fixed key (`utils.HighwayHashFile`).
pub fn highway_hash_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use highway::HighwayHash;
    const KEY_HEX: [u64; 4] = [
        u64::from_le_bytes([0x15, 0x53, 0xc5, 0x38, 0x3f, 0xb0, 0xb8, 0x65]),
        u64::from_le_bytes([0x78, 0xc3, 0x31, 0x0d, 0xa6, 0x65, 0xb4, 0xf6]),
        u64::from_le_bytes([0xe0, 0x52, 0x1a, 0xcf, 0x22, 0xeb, 0x58, 0xa9]),
        u64::from_le_bytes([0x95, 0x32, 0xff, 0xed, 0x02, 0xa6, 0xb1, 0x15]),
    ];
    let mut hasher = highway::HighwayHasher::new(highway::Key(KEY_HEX));
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.append(&buf[..n]);
    }
    let out = hasher.finalize256();
    let mut bytes = Vec::with_capacity(32);
    for word in out {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    Ok(bytes)
}

/// Hash a file with the named algorithm, mirroring `utils.HashFile`.
/// `xxhash` (croc's default) produces the 8-byte big-endian XXH64 sum, like
/// Go's `cespare/xxhash` `Sum(nil)`.
pub fn hash_file(path: &Path, algorithm: &str) -> std::io::Result<Vec<u8>> {
    match algorithm {
        "imohash" => return imohash_file(path),
        "highway" => return highway_hash_file(path),
        _ => {}
    }
    match algorithm {
        "xxhash" => {
            let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
            let mut f = std::fs::File::open(path)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.digest().to_be_bytes().to_vec())
        }
        "md5" => {
            let mut hasher = Md5::new();
            let mut f = std::fs::File::open(path)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize().to_vec())
        }
        other => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("unknown hash algorithm '{other}' (use xxhash, imohash, highway, or md5)"),
        )),
    }
}

/// Ports with nothing listening, mirroring `utils.FindOpenPorts`
/// (a port is "open" for our use when dialing it fails).
pub fn find_open_ports(host: &str, port_start: u16, num_ports: usize) -> Vec<u16> {
    let mut open = Vec::new();
    for port in port_start..port_start.saturating_add(200) {
        let addr: std::net::SocketAddr = match format!("{host}:{port}").parse() {
            Ok(a) => a,
            Err(_) => break,
        };
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(100))
            .is_err()
        {
            open.push(port);
        }
        if open.len() >= num_ports {
            break;
        }
    }
    open
}

/// Whether `address` (host or host:port) is loopback, link-local, or in a
/// private RFC1918/ULA range — mirroring `utils.IsLocalIP`. Proxies are
/// skipped for such addresses.
pub fn is_local_ip(address: &str) -> bool {
    use std::net::IpAddr;
    if address.contains("127.0.0.1") {
        return true;
    }
    // Strip an optional :port (also handles [v6]:port).
    let host = match address.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => h,
        _ => address,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return false,
    };
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_link_local()
                || o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            // fe80::/10 link-local, fc00::/7 unique-local
            (seg[0] & 0xffc0) == 0xfe80 || (seg[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Non-loopback IPv4 addresses of this host, mirroring `utils.GetLocalIPs`.
pub fn get_local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let std::net::IpAddr::V4(v4) = iface.ip() {
                ips.push(v4.to_string());
            }
        }
    }
    ips
}

/// Find all-zero chunks of an on-disk file, encoded as croc chunk ranges:
/// `[chunkSize, start1, count1, start2, count2, ...]`. Mirrors
/// `utils.MissingChunks` (used to resume partially transferred files).
pub fn missing_chunks(path: &Path, fsize: i64, chunk_size: usize) -> Vec<i64> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    match f.metadata() {
        Ok(m) if m.len() as i64 == fsize => {}
        _ => return Vec::new(),
    }

    let empty = vec![0u8; chunk_size];
    let mut chunks: Vec<i64> = Vec::new();
    let mut current_location: i64 = 0;
    let mut buffer = vec![0u8; chunk_size];
    loop {
        // Mirror Go's f.Read: one read per chunk, stopping at EOF/short file.
        let n = match f.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if buffer[..n] == empty[..n] {
            chunks.push(current_location);
        }
        current_location += n as i64;
    }

    if chunks.is_empty() {
        return Vec::new();
    }
    let mut ranges = vec![chunk_size as i64, chunks[0]];
    let mut cur_count = 0i64;
    for i in 1..chunks.len() {
        cur_count += 1;
        if chunks[i] - chunks[i - 1] > chunk_size as i64 {
            ranges.push(cur_count);
            ranges.push(chunks[i]);
            cur_count = 0;
        }
    }
    ranges.push(cur_count + 1);
    ranges
}

/// Expand chunk ranges back into chunk start positions. Mirrors
/// `utils.ChunkRangesToChunks`.
pub fn chunk_ranges_to_chunks(chunk_ranges: &[i64]) -> Vec<i64> {
    if chunk_ranges.is_empty() {
        return Vec::new();
    }
    let chunk_size = chunk_ranges[0];
    let mut chunks = Vec::new();
    let mut i = 1;
    while i + 1 < chunk_ranges.len() {
        for j in 0..chunk_ranges[i + 1] {
            chunks.push(chunk_ranges[i] + j * chunk_size);
        }
        i += 2;
    }
    chunks
}

/// Human-readable byte count, mirroring `utils.ByteCountDecimal`
/// (which despite its name uses 1024 units).
pub fn byte_count_decimal(b: i64) -> String {
    const UNIT: i64 = 1024;
    if b < UNIT {
        return format!("{b} B");
    }
    let mut div = UNIT;
    let mut exp = 0;
    let mut n = b / UNIT;
    while n >= UNIT {
        div *= UNIT;
        exp += 1;
        n /= UNIT;
    }
    format!(
        "{:.1} {}B",
        b as f64 / div as f64,
        ['k', 'M', 'G', 'T', 'P', 'E'][exp]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_shape() {
        let pin = generate_random_pin();
        assert_eq!(pin.len(), NB_PIN_NUMBERS);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn name_shape() {
        let name = get_random_name();
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 1 + mnemonicode::words_required(NB_BYTES_WORDS));
    }

    // Vectors from Go: utils.XXHashFile / utils.MD5HashFile of
    // "the quick brown fox jumps over the lazy dog".
    #[test]
    fn hash_vectors_match_go() {
        let dir = std::env::temp_dir().join("rusty-croc-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("xxtest.bin");
        std::fs::write(&p, b"the quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(
            hex::encode(hash_file(&p, "xxhash").unwrap()),
            "ed714233c5a9a792"
        );
        assert_eq!(
            hex::encode(hash_file(&p, "md5").unwrap()),
            "77add1d5f41223d5582fca736a5cb335"
        );
    }

    // Vectors from Go: utils.IMOHashFile / utils.HighwayHashFile over
    // deterministic content (small = full-hash path, big = sampled path).
    #[test]
    fn imohash_highway_vectors_match_go() {
        let dir = std::env::temp_dir().join("rusty-croc-imo-test");
        std::fs::create_dir_all(&dir).unwrap();
        let small: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let big: Vec<u8> = (0..9 * 1024 * 1024u64)
            .map(|i| ((i * 7) % 253) as u8)
            .collect();
        let ps = dir.join("small.bin");
        let pb = dir.join("big.bin");
        std::fs::write(&ps, &small).unwrap();
        std::fs::write(&pb, &big).unwrap();
        assert_eq!(
            hex::encode(imohash_file(&ps).unwrap()),
            "e80762416fa3a01c3a12c3f073f1099e"
        );
        assert_eq!(
            hex::encode(imohash_file(&pb).unwrap()),
            "8080c00496dd707f36645be7f15df6f4"
        );
        assert_eq!(
            hex::encode(highway_hash_file(&ps).unwrap()),
            "e417fdbe375b8b83f33e4168cc64d621d13c6a107250935e60e0ee0654c48d1a"
        );
        assert_eq!(
            hex::encode(highway_hash_file(&pb).unwrap()),
            "451d67cac71669ce99505502ca8a688d41cc5145af4344873aa5f47ee6e624e9"
        );
    }

    #[test]
    fn find_open_ports_returns_requested_count() {
        let ports = find_open_ports("127.0.0.1", 39000, 3);
        assert_eq!(ports.len(), 3);
        assert!(ports[0] >= 39000);
    }

    #[test]
    fn chunk_ranges_round_trip() {
        // ranges: chunkSize=4, runs starting at 0 (2 chunks) and 16 (1 chunk)
        let ranges = vec![4, 0, 2, 16, 1];
        assert_eq!(chunk_ranges_to_chunks(&ranges), vec![0, 4, 16]);
        assert_eq!(chunk_ranges_to_chunks(&[]), Vec::<i64>::new());
    }

    #[test]
    fn missing_chunks_detects_zero_runs() {
        let dir = std::env::temp_dir().join("rusty-croc-chunk-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("partial.bin");
        // 4 chunks of 4 bytes: data, zeros, zeros, data
        let mut content = Vec::new();
        content.extend_from_slice(b"aaaa");
        content.extend_from_slice(&[0u8; 8]);
        content.extend_from_slice(b"bbbb");
        std::fs::write(&p, &content).unwrap();
        let ranges = missing_chunks(&p, 16, 4);
        assert_eq!(chunk_ranges_to_chunks(&ranges), vec![4, 8]);
    }
}
