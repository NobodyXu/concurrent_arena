use core::cell::UnsafeCell;
use core::mem::size_of;
use core::ops::Deref;

use std::sync::atomic::{fence, AtomicU8, Ordering};
use std::sync::Arc;

use parking_lot::RawThreadId;

use bitvec::access::BitSafeUsize;
use bitvec::prelude::*;

use array_init::array_init;

struct Entry<T> {
    counter: AtomicU8,
    val: UnsafeCell<Option<T>>,
}

impl<T> Default for Entry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Entry<T> {
    fn new() -> Self {
        Self {
            counter: AtomicU8::new(0),
            val: UnsafeCell::new(None),
        }
    }
}

/// * `LEN` - Must be less than or equal to `u32::MAX`
/// * `BITARRAY_LEN` - Must be equal to `LEN / core::mem::size_of::<usize>()`.
pub(crate) struct Bucket<T, const BITARRAY_LEN: usize, const LEN: usize> {
    bitset: BitArray<Lsb0, [BitSafeUsize; BITARRAY_LEN]>,
    entries: [Entry<T>; LEN],
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Bucket<T, BITARRAY_LEN, LEN> {
    pub(crate) fn new() -> Self {
        if LEN > (u32::MAX as usize) {
            panic!("LEN must be no larger than u32::MAX {}", u32::MAX);
        }
        if LEN / size_of::<usize>() != BITARRAY_LEN {
            panic!("BITARRAY_LEN MUST be equal to LEN / core::mem::size_of::<usize>()");
        }

        Self {
            bitset: BitArray::zeroed(),
            entries: array_init(|_| Entry::new()),
        }
    }

    pub(crate) fn insert(&self, bucket_index: u32) -> ArenaArc<T, BITARRAY_LEN, LEN> {
        todo!()
    }
}

/// Can have at most u8::MAX refcount.
pub struct ArenaArc<T, const BITARRAY_LEN: usize, const LEN: usize> {
    slot: u32,
    bucket: Arc<Bucket<T, BITARRAY_LEN, LEN>>,
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> ArenaArc<T, BITARRAY_LEN, LEN> {
    pub fn slot(&self) -> u32 {
        self.slot
    }

    fn get_entry(&self) -> &Entry<T> {
        let entry = &self.bucket.entries[(self.slot as usize) % LEN];
        debug_assert!(entry.counter.load(Ordering::Relaxed) > 0);
        entry
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Deref for ArenaArc<T, BITARRAY_LEN, LEN> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = self.get_entry().val.get() as *const T;

        unsafe { &*ptr }
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Clone for ArenaArc<T, BITARRAY_LEN, LEN> {
    fn clone(&self) -> Self {
        let entry = self.get_entry();

        // According to [Boost documentation][1], increasing the refcount
        // can be done using Relaxed operation since there are at least one
        // reference alive.
        //
        // [1]: https://www.boost.org/doc/libs/1_77_0/doc/html/atomic/usage_examples.html
        if entry.counter.fetch_add(1, Ordering::Relaxed) == u8::MAX {
            panic!("ArenaArc can have at most u8::MAX refcount");
        }

        Self {
            slot: self.slot,
            bucket: Arc::clone(&self.bucket),
        }
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Drop for ArenaArc<T, BITARRAY_LEN, LEN> {
    fn drop(&mut self) {
        let entry = self.get_entry();

        // According to [Boost documentation][1], decreasing refcount must be done
        // using Release to ensure the write to the value happens before the
        // reference is dropped.
        //
        // [1]: https://www.boost.org/doc/libs/1_77_0/doc/html/atomic/usage_examples.html
        if entry.counter.fetch_sub(1, Ordering::Release) == 1 {
            // This is the last reference, drop the value.

            // According to [Boost documentation][1], an Acquire fence must be used
            // before dropping value to ensure that all write to the value happens
            // before it is dropped.
            fence(Ordering::Acquire);

            // Now entry.counter == 0
            unsafe { entry.val.get().drop_in_place() };
        }
    }
}
