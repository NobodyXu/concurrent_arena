use super::bucket::Bucket;

use std::sync::Arc;

use parking_lot::RwLock;

/// * `LEN` - Must be less than or equal to `u32::MAX`, divisible by
///   `usize::BITS` and it must not be `0`.
/// * `BITARRAY_LEN` - Must be equal to `LEN / usize::BITS`.
///
/// `Arena` stores the elements in buckets to ensure that the address
/// for elements are stable while improving efficiency.
///
/// Every bucket is of size `LEN`.
///
/// The larger `LEN` is, the more compact the `Arena` will be, however it might
/// also waste space if it is unused.
///
/// And, allocating a large chunk of memory takes more time.
#[derive(Debug)]
pub struct Arena<T, const BITARRAY_LEN: usize, const LEN: usize> {
    buckets: RwLock<Vec<Arc<Bucket<T, BITARRAY_LEN, LEN>>>>,
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Default for Arena<T, BITARRAY_LEN, LEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Arena<T, BITARRAY_LEN, LEN> {
    /// Would preallocate 2 buckets.
    pub fn new() -> Self {
        Self::with_capacity(2)
    }

    pub fn with_capacity(cap: u32) -> Self {
        let bits = usize::BITS as usize;

        if LEN > (u32::MAX as usize) {
            panic!("LEN must be no larger than u32::MAX {}", u32::MAX);
        }
        if LEN / bits != BITARRAY_LEN {
            panic!("BITARRAY_LEN MUST be equal to LEN / usize::BITS");
        }

        if LEN % bits != 0 {
            panic!("bitarray_LEN MUST be divisible usize::BITS");
        }

        if LEN == 0 {
            panic!("LEN must not be 0");
        }

        let mut buckets = Vec::with_capacity(cap as usize);
        for _ in 0..cap {
            buckets.push(Arc::new(Bucket::new()));
        }

        Self {
            buckets: RwLock::new(buckets),
        }
    }
}
