use crate::{decrypt, encrypt};

/// Wrap payload in N layers of onion encryption.
/// Each relay peels one layer to reveal the inner payload for the next hop.
/// No magic header — the output is indistinguishable from random bytes to DPI.
pub fn onion_wrap(payload: &[u8], hop_keys: &[[u8; 32]]) -> Vec<u8> {
    let mut wrapped = payload.to_vec();

    for (index, key) in hop_keys.iter().enumerate().rev() {
        let remaining_hops = (hop_keys.len() - index) as u8;
        // Place hop metadata INSIDE the encrypted envelope so it is invisible to DPI.
        let mut inner = Vec::with_capacity(1 + wrapped.len());
        inner.push(remaining_hops);
        inner.extend_from_slice(&wrapped);
        // encrypt() prepends a random 24-byte nonce — the first bytes are always random.
        wrapped = encrypt(key, &inner);
    }

    wrapped
}

/// Peel one layer of onion encryption.
/// Returns `(remaining_hops, inner_payload)`.
/// `remaining_hops == 1` means this relay is the last hop and `inner_payload` is the real data.
pub fn onion_peel(layer: &[u8], hop_key: &[u8; 32]) -> Result<(u8, Vec<u8>), &'static str> {
    let decrypted = decrypt(hop_key, layer)?;
    if decrypted.is_empty() {
        return Err("empty onion layer");
    }
    let remaining_hops = decrypted[0];
    let inner = decrypted[1..].to_vec();
    Ok((remaining_hops, inner))
}

/// Legacy peel that returns only the inner payload (for backward-compatible call sites).
pub fn onion_peel_payload(layer: &[u8], hop_key: &[u8; 32]) -> Result<Vec<u8>, &'static str> {
    let (_remaining, inner) = onion_peel(layer, hop_key)?;
    Ok(inner)
}
