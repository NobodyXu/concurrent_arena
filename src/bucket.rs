use super::bitmap::BitMap;

use core::cell::UnsafeCell;
use core::mem::size_of;
use core::ops::Deref;

use std::sync::atomic::{fence, AtomicU8, Ordering};
use std::sync::Arc;

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

/// * `LEN` - Must be less than or equal to `u32::MAX`, divisible by
///   `core::mem::size_of::<usize>()` and it must not be `0`.
/// * `BITARRAY_LEN` - Must be equal to `LEN / core::mem::size_of::<usize>()`.
pub(crate) struct Bucket<T, const BITARRAY_LEN: usize, const LEN: usize> {
    bitset: BitMap<BITARRAY_LEN>,
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

        if LEN % size_of::<usize>() != 0 {
            panic!("bitarray_LEN MUST be divisible core::mem::size_of::<usize>()");
        }

        if LEN == 0 {
            panic!("LEN must not be 0");
        }

        Self {
            bitset: BitMap::new(),
            entries: array_init(|_| Entry::new()),
        }
    }

    pub(crate) fn insert(
        this: &Arc<Self>,
        bucket_index: u32,
        value: T,
    ) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let index = this.bitset.allocate()?;

        // Make sure drop is written to memory before
        // the entry is reused again.
        fence(Ordering::Acquire);

        let entry = &this.entries[index];

        let prev_refcnt = entry.counter.fetch_add(1, Ordering::Relaxed);
        debug_assert_eq!(prev_refcnt, 0);

        let option = unsafe { &mut *entry.val.get() };
        debug_assert!(option.is_none());
        *option = Some(value);

        Some(ArenaArc {
            slot: bucket_index * (LEN as u32) + index as u32,
            bucket: Arc::clone(this),
        })
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

    fn get_index(&self) -> usize {
        (self.slot as usize) % LEN
    }

    fn get_entry(&self) -> &Entry<T> {
        let entry = &self.bucket.entries[self.get_index()];
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
        let prev_refcnt = entry.counter.fetch_sub(1, Ordering::Release);

        debug_assert_ne!(prev_refcnt, 0);

        if prev_refcnt == 1 {
            // This is the last reference, drop the value.

            // According to [Boost documentation][1], an Acquire fence must be used
            // before dropping value to ensure that all write to the value happens
            // before it is dropped.
            fence(Ordering::Acquire);

            // Now entry.counter == 0
            let option = unsafe { &mut *entry.val.get() };
            *option = None;

            // Make sure drop is written to memory before
            // the entry is reused again.
            fence(Ordering::Release);

            self.bucket.bitset.deallocate(self.get_index());
        }
    }
}
