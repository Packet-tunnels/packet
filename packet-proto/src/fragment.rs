use rand::Rng;
use serde::{Deserialize, Serialize};

const FIELD_PRIME: i32 = 257;

/// Standard fragment sizes for anti-fingerprinting.
/// All fragments are padded to one of these sizes so DPI cannot distinguish
/// text fragments from media fragments.
pub const STANDARD_SIZES: [usize; 4] = [512, 4096, 65536, 524288];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KOfNFragment {
    pub set_id: String,
    pub threshold: u8,
    pub share_index: u8,
    pub total_shares: u8,
    pub payload_len: usize,
    pub payload_hash: String,
    pub share_data: Vec<u16>,
}

impl KOfNFragment {
    /// Serialize to bytes for network transport, padded to a standard size.
    pub fn to_padded_bytes(&self) -> Vec<u8> {
        let raw = serde_json::to_vec(self).unwrap_or_default();
        let target = next_standard_size(raw.len() + 4); // 4 bytes for length prefix
        let mut out = Vec::with_capacity(target);
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&raw);
        // Pad with random bytes (not zeros — zeros are detectable)
        let mut rng = rand::thread_rng();
        while out.len() < target {
            out.push(rng.gen());
        }
        out
    }

    /// Deserialize from padded bytes.
    pub fn from_padded_bytes(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 4 {
            return Err("fragment too short");
        }
        let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + len {
            return Err("fragment truncated");
        }
        serde_json::from_slice(&data[4..4 + len]).map_err(|_| "fragment deserialize failed")
    }
}

pub fn split_secret_shares(
    payload: &[u8],
    threshold: u8,
    total_shares: u8,
) -> Result<Vec<KOfNFragment>, &'static str> {
    if payload.is_empty() {
        return Err("payload empty");
    }
    if threshold < 2 {
        return Err("threshold too small");
    }
    if total_shares < threshold {
        return Err("total shares smaller than threshold");
    }

    let set_id = crate::generate_auth_nonce();
    let payload_hash = crate::hex::encode(&crate::sha256(payload));
    let mut share_data = vec![vec![0u16; payload.len()]; total_shares as usize];
    let mut rng = rand::thread_rng();

    for (offset, byte) in payload.iter().enumerate() {
        let mut coefficients = Vec::with_capacity(threshold as usize);
        coefficients.push(i32::from(*byte));
        for _ in 1..threshold {
            coefficients.push(rng.gen_range(0..FIELD_PRIME) as i32);
        }

        for share_idx in 1..=total_shares {
            let x = i32::from(share_idx);
            let mut y = 0i32;
            let mut power = 1i32;
            for coefficient in &coefficients {
                y = mod_prime(y + coefficient * power);
                power = mod_prime(power * x);
            }
            share_data[(share_idx - 1) as usize][offset] = y as u16;
        }
    }

    Ok((1..=total_shares)
        .map(|share_index| KOfNFragment {
            set_id: set_id.clone(),
            threshold,
            share_index,
            total_shares,
            payload_len: payload.len(),
            payload_hash: payload_hash.clone(),
            share_data: share_data[(share_index - 1) as usize].clone(),
        })
        .collect())
}

pub fn reconstruct_secret(fragments: &[KOfNFragment]) -> Result<Vec<u8>, &'static str> {
    if fragments.is_empty() {
        return Err("no fragments");
    }

    let first = &fragments[0];
    let threshold = first.threshold as usize;
    if threshold < 2 {
        return Err("invalid fragment threshold");
    }
    if fragments.len() < threshold {
        return Err("insufficient fragments");
    }
    if first.total_shares < first.threshold || first.share_index == 0 {
        return Err("invalid fragment metadata");
    }

    let mut seen_indices = [false; u8::MAX as usize + 1];

    for fragment in fragments {
        if fragment.set_id != first.set_id
            || fragment.threshold != first.threshold
            || fragment.total_shares != first.total_shares
            || fragment.payload_len != first.payload_len
            || fragment.payload_hash != first.payload_hash
            || fragment.share_data.len() != first.payload_len
            || fragment.share_index == 0
            || fragment.share_index > fragment.total_shares
            || fragment
                .share_data
                .iter()
                .any(|value| usize::from(*value) >= FIELD_PRIME as usize)
        {
            return Err("incompatible fragments");
        }

        let index = fragment.share_index as usize;
        if seen_indices[index] {
            return Err("duplicate fragment share index");
        }
        seen_indices[index] = true;
    }

    // FAST PATH: try the first k fragments (most common case — no corruption)
    let fast_subset: Vec<&KOfNFragment> = fragments.iter().take(threshold).collect();
    if let Ok(payload) = reconstruct_subset(&fast_subset) {
        let hash = crate::hex::encode(&crate::sha256(&payload));
        if hash == first.payload_hash {
            return Ok(payload);
        }
    }

    // SLOW PATH: some fragment may be corrupted, try other combinations
    // Cap at 100 attempts to prevent explosion with large n
    let combinations = choose_indices(fragments.len(), threshold);
    for combination in combinations.into_iter().skip(1).take(100) {
        let subset: Vec<_> = combination.iter().map(|&idx| &fragments[idx]).collect();
        if let Ok(payload) = reconstruct_subset(&subset) {
            let hash = crate::hex::encode(&crate::sha256(&payload));
            if hash == first.payload_hash {
                return Ok(payload);
            }
        }
    }

    Err("fragment checksum mismatch")
}

fn reconstruct_subset(fragments: &[&KOfNFragment]) -> Result<Vec<u8>, &'static str> {
    let payload_len = fragments[0].payload_len;
    let mut payload = Vec::with_capacity(payload_len);

    for offset in 0..payload_len {
        let mut secret = 0i32;
        for (share_pos, fragment) in fragments.iter().enumerate() {
            let xi = i32::from(fragment.share_index);
            let yi = i32::from(fragment.share_data[offset]);
            let mut numerator = 1i32;
            let mut denominator = 1i32;

            for (other_pos, other_fragment) in fragments.iter().enumerate() {
                if share_pos == other_pos {
                    continue;
                }
                let xj = i32::from(other_fragment.share_index);
                numerator = mod_prime(numerator * -xj);
                denominator = mod_prime(denominator * (xi - xj));
            }

            let inv = mod_inverse(denominator).ok_or("fragment interpolation failure")?;
            let basis = mod_prime(numerator * inv);
            secret = mod_prime(secret + yi * basis);
        }

        if !(0..=255).contains(&secret) {
            return Err("fragment produced invalid byte");
        }
        payload.push(secret as u8);
    }

    Ok(payload)
}

fn choose_indices(total: usize, threshold: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut current = Vec::with_capacity(threshold);
    choose_indices_recursive(0, total, threshold, &mut current, &mut out);
    out
}

fn choose_indices_recursive(
    start: usize,
    total: usize,
    threshold: usize,
    current: &mut Vec<usize>,
    out: &mut Vec<Vec<usize>>,
) {
    if current.len() == threshold {
        out.push(current.clone());
        return;
    }

    for index in start..total {
        current.push(index);
        choose_indices_recursive(index + 1, total, threshold, current, out);
        current.pop();
    }
}

fn mod_inverse(value: i32) -> Option<i32> {
    let mut t = 0i32;
    let mut new_t = 1i32;
    let mut r = FIELD_PRIME;
    let mut new_r = mod_prime(value);

    while new_r != 0 {
        let quotient = r / new_r;
        let tmp_t = t - quotient * new_t;
        t = new_t;
        new_t = tmp_t;
        let tmp_r = r - quotient * new_r;
        r = new_r;
        new_r = tmp_r;
    }

    if r > 1 {
        return None;
    }

    Some(mod_prime(t))
}

fn mod_prime(value: i32) -> i32 {
    let mut result = value % FIELD_PRIME;
    if result < 0 {
        result += FIELD_PRIME;
    }
    result
}

/// Pick the next standard fragment size that can hold `data_len` bytes.
fn next_standard_size(data_len: usize) -> usize {
    for &size in &STANDARD_SIZES {
        if size >= data_len {
            return size;
        }
    }
    // Larger than biggest standard — round up to next 512KB boundary
    ((data_len + 524287) / 524288) * 524288
}
