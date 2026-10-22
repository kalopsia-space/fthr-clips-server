use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

const CHUNK: usize = 1024 * 1024;
const SIDE: u64 = 16 * 1024 * 1024;
const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn hash_n(hasher: &mut Xxh3, file: &mut File, mut remaining: u64) -> io::Result<()> {
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = remaining.min(CHUNK as u64) as usize;
        file.read_exact(&mut buf[..want])?;
        hasher.update(&buf[..want]);
        remaining -= want as u64;
    }
    Ok(())
}

/// Content identity: xxh3-128 over the whole file (<= 32 MiB) or its first and
/// last 16 MiB, then the little-endian size. Returns 32 lowercase hex chars.
pub fn media_id(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Xxh3::new();
    if size <= 2 * SIDE {
        hash_n(&mut hasher, &mut file, size)?;
    } else {
        hash_n(&mut hasher, &mut file, SIDE)?;
        file.seek(SeekFrom::Start(size - SIDE))?;
        hash_n(&mut hasher, &mut file, SIDE)?;
    }
    hasher.update(&size.to_le_bytes());
    Ok(hex::encode(hasher.digest128().to_be_bytes()))
}

fn base62(mut n: u64) -> String {
    let mut out = [b'0'; 11];
    for slot in out.iter_mut().rev() {
        *slot = BASE62[(n % 62) as usize];
        n /= 62;
    }
    String::from_utf8(out.to_vec()).expect("base62 alphabet is ascii")
}

/// Slug candidates for a media id: high half, low half, both concatenated.
/// Precondition: `id` is 32 hex characters (as produced by `media_id`).
pub fn slug_candidates(id: &str) -> [String; 3] {
    assert!(
        id.len() == 32 && id.is_ascii(),
        "media id must be 32 hex chars"
    );
    let hi = u64::from_str_radix(&id[..16], 16).expect("media id must be hex");
    let lo = u64::from_str_radix(&id[16..], 16).expect("media id must be hex");
    let (a, b) = (base62(hi), base62(lo));
    let both = format!("{a}{b}");
    [a, b, both]
}
