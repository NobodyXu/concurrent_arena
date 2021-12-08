use super::bucket::Bucket;
use super::ArenaArc;

use core::cmp::min;

use std::sync::Arc;

use arrayvec::ArrayVec;

use parking_lot::const_rwlock;
use parking_lot::lock_api::GetThreadId;
use parking_lot::RawThreadId;
use parking_lot::RwLock;
use parking_lot::RwLockUpgradableReadGuard;

use const_fn_assert::{cfn_assert, cfn_assert_eq, cfn_assert_ne};

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
    const fn check_const_generics() {
        let bits = usize::BITS as usize;

        cfn_assert!(LEN <= (u32::MAX as usize));
        cfn_assert_eq!(LEN % bits, 0);
        cfn_assert_ne!(LEN, 0);

        cfn_assert_eq!(LEN / bits, BITARRAY_LEN);
    }

    /// Maximum buckets `Arena` can have.
    pub const fn max_buckets() -> u32 {
        Self::check_const_generics();

        u32::MAX / (LEN as u32)
    }

    /// Would preallocate 2 buckets.
    pub fn new() -> Self {
        Self::with_capacity(2)
    }

    pub const fn const_new() -> Self {
        Self::check_const_generics();

        Self {
            buckets: const_rwlock(Vec::new()),
        }
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

    /// * `BUFFER_SIZE` - Must be less than or equal to `Self::max_buckets()` and
    ///   greater than 0.
    ///
    ///   It will be used to store `Arc` (8 bytes on 64-bit platform and
    ///   4 bytes on 32-bit platform).
    ///
    ///   It is suggested to use any value between [10, 30] and less than or equal to
    ///   `new_len - len`.
    ///
    /// Reserve `min(new_len, Self::max_buckets())` buckets.
    pub fn reserve<const BUFFER_SIZE: usize>(&self, new_len: u32) {
        cfn_assert!(BUFFER_SIZE <= Self::max_buckets() as usize);
        cfn_assert!(BUFFER_SIZE > 0);

        let new_len = min(new_len, Self::max_buckets());
        let mut buffer = ArrayVec::<Arc<Bucket<T, BITARRAY_LEN, LEN>>, BUFFER_SIZE>::new();

        loop {
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
            //
            // First, we allocate new bucket and put it in buffer
            // to avoid blocking the readers.
            let cnt = new_len - len;

            for _ in 0..min(cnt, BUFFER_SIZE as u32) {
                buffer.try_push(Arc::new(Bucket::new())).unwrap();
            }

            // Push all allocated buckets into the buffer at once.
            {
                let mut guard = RwLockUpgradableReadGuard::upgrade(guard);
                // Drain the buffer
                for new_bucket in buffer.drain(..) {
                    guard.push(new_bucket);
                }
            }

            if cnt <= BUFFER_SIZE as u32 {
                break;
            }
        }
    }

    pub fn insert(&self, mut value: T) -> ArenaArc<T, BITARRAY_LEN, LEN> {
        loop {
            match self.try_insert(value) {
                Ok(arc) => break arc,
                Err((val, len)) => {
                    value = val;

                    if len != Self::max_buckets() {
                        self.reserve::<1>(len + 1);
                    }
                }
            }
        }
    }

    /// May enter busy loop if the slot is not fully initialized.
    pub fn remove(&self, slot: u32) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let bucket_index = slot / (LEN as u32);
        let index = slot % (LEN as u32);

        let bucket = self.buckets.read()[bucket_index as usize].clone();

        Bucket::remove(bucket, bucket_index, index)
    }

    /// Return number of buckets allocated.
    pub fn len(&self) -> u32 {
        self.buckets.read().len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
