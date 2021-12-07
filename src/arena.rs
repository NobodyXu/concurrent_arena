use super::bucket::Bucket;

use std::sync::Arc;

use parking_lot::RwLock;

/// * `LEN` - Must be less than or equal to `u32::MAX`, divisible by
///   `core::mem::size_of::<usize>()` and it must not be `0`.
/// * `BITARRAY_LEN` - Must be equal to `LEN / core::mem::size_of::<usize>()`.
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

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Arena<T, BITARRAY_LEN, LEN> {
    pub fn new() -> Self {
        todo!()
    }
}
