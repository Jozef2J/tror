//! Arithmetic on share vectors: relative weights, one per child of a linear node, or one per
//! column or row of a grid.

/// The sum of the shares.
pub(crate) fn total(shares: &[f32]) -> f32 {
    shares.iter().sum()
}

/// The part of the space (0 to 1) the share at `index` gets; an even part if the shares sum
/// to 0.
pub(crate) fn fraction(shares: &[f32], index: usize) -> f32 {
    let total = total(shares);
    if total > 0.0 {
        shares[index] / total
    } else {
        1.0 / shares.len() as f32
    }
}

/// Where the boundary after the share at `index` stands, from 0 to 1.
pub(crate) fn boundary(shares: &[f32], index: usize) -> f32 {
    (0..=index).map(|i| fraction(shares, i)).sum()
}

/// Scales the shares so they sum to 1, or makes them equal if they sum to 0.
pub(crate) fn normalize(shares: &mut [f32]) {
    let total = total(shares);
    let n = shares.len() as f32;
    for share in shares.iter_mut() {
        *share = if total > 0.0 { *share / total } else { 1.0 / n };
    }
}

/// Moves the boundary after the share at `index` by `delta`, a part of the whole (0 to 1),
/// between that share and the next one. Each keeps at least `min`, a part of the whole too.
/// The other shares keep their size.
pub(crate) fn move_boundary(shares: &mut [f32], index: usize, delta: f32, min: f32) {
    normalize(shares);
    let pair = shares[index] + shares[index + 1];
    let min = min.min(pair / 2.0);
    let first = (shares[index] + delta).clamp(min, pair - min);
    shares[index] = first;
    shares[index + 1] = pair - first;
}

/// Gives the share at `index` and the next one the same size.
pub(crate) fn even_pair(shares: &mut [f32], index: usize) {
    normalize(shares);
    let half = (shares[index] + shares[index + 1]) / 2.0;
    shares[index] = half;
    shares[index + 1] = half;
}

/// Brings `shares` to `len` entries: extra ones are dropped, new ones get the mean of the
/// others (or 1). Shares that are negative or not finite become 0.
pub(crate) fn fit(shares: &mut Vec<f32>, len: usize) {
    for share in shares.iter_mut() {
        if !share.is_finite() || *share < 0.0 {
            *share = 0.0;
        }
    }
    if shares.len() > len {
        shares.truncate(len);
    } else if shares.len() < len {
        let mean = if shares.is_empty() || total(shares) <= 0.0 {
            1.0
        } else {
            total(shares) / shares.len() as f32
        };
        shares.resize(len, mean);
    }
}
