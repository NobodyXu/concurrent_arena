use super::bucket::Bucket;
use super::ArenaArc;

use core::cmp::min;

use std::sync::Arc;

use parking_lot::lock_api::GetThreadId;
use parking_lot::RawThreadId;
use parking_lot::RwLock;
use parking_lot::RwLockUpgradableReadGuard;

/// * `LEN` - Number of elements stored per bucket.
///    Must be less than or equal to `u32::MAX`, divisible by
///   `usize::BITS` and it must not be `0`.
/// * `BITARRAY_LEN` - Number bits in the bitmap per bucket.
///   Must be equal to `LEN / usize::BITS`.
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
    fn check_const_generics() {
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
    }

    /// Maximum buckets `Arena` can have.
    pub fn max_buckets() -> u32 {
        Self::check_const_generics();

        u32::MAX / (LEN as u32)
    }

    /// Would preallocate 2 buckets.
    pub fn new() -> Self {
        Self::with_capacity(2)
    }

    pub fn with_capacity(cap: u32) -> Self {
        Self::check_const_generics();

        let cap = min(cap, Self::max_buckets());

        let mut buckets = Vec::with_capacity(cap as usize);
        for _ in 0..cap {
            buckets.push(Arc::new(Bucket::new()));
        }

        Self {
            buckets: RwLock::new(buckets),
        }
    }

    fn try_insert(&self, mut value: T) -> Result<ArenaArc<T, BITARRAY_LEN, LEN>, (T, u32)> {
        let guard = self.buckets.read();
        let len = guard.len();

        debug_assert!(len <= u32::MAX as usize);
        debug_assert!(len <= Self::max_buckets() as usize);

        let mut pos = RawThreadId::INIT.nonzero_thread_id().get() % len;

        let slice1_iter = guard[pos..].iter();
        let slice2_iter = guard[..pos].iter();

        for bucket in slice1_iter.chain(slice2_iter) {
            match Bucket::try_insert(bucket, pos as u32, value) {
                Ok(arc) => return Ok(arc),
                Err(val) => value = val,
            }

            pos = (pos + 1) % len;
        }

        Err((value, len as u32))
    }

    pub fn reserve(&self, new_len: u32) {
        let new_len = min(new_len, Self::max_buckets());

        // Use an upgradable_read to check if the key has already
        // been added by another thread.
        //
        // Unlike write guard, this UpgradableReadGuard only blocks
        // other UpgradableReadGuard and WriteGuard, so the readers
        // will not be blocked while ensuring that there is no other
        // writer.
        let guard = self.buckets.upgradable_read();
        let len = guard.len() as u32;

        // If another writer has already done the reservation, return.
        if len >= new_len {
            return;
        }

        // If no other writer has done the reservation, do it now.
        let mut guard = RwLockUpgradableReadGuard::upgrade(guard);
        for _ in len..new_len {
            guard.push(Arc::new(Bucket::new()));
        }
    }

    pub fn insert(&self, mut value: T) -> ArenaArc<T, BITARRAY_LEN, LEN> {
        loop {
            match self.try_insert(value) {
                Ok(arc) => break arc,
                Err((val, len)) => {
                    value = val;

                    if len != u32::MAX {
                        self.reserve(len + 1);
                    }
                }
            }
        }
    }

    pub fn remove(&self, slot: u32) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let bucket_index = slot / (LEN as u32);
        let index = slot % (LEN as u32);

        Bucket::remove(
            &self.buckets.read()[bucket_index as usize],
            bucket_index,
            index,
        )
    }
}
