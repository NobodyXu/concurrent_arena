#![forbid(unsafe_code)]

use core::{marker::PhantomData, ops::Deref, slice::Iter};

use arc_swap::{ArcSwapAny, Guard};
use parking_lot::Mutex;

use triomphe::ThinArc;

#[derive(Debug)]
pub(crate) struct Arcs<T> {
    array: ArcSwapAny<Option<ThinArc<(), T>>>,
    mutex: Mutex<()>,
}

impl<T> Arcs<T> {
    pub(crate) fn new() -> Self {
        Self {
            array: ArcSwapAny::new(None),
            mutex: Mutex::new(()),
        }
    }

    pub(crate) fn as_slice(&self) -> Slice<'_, T> {
        Slice(self.array.load(), PhantomData)
    }

    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Clone> Arcs<T> {
    pub(crate) fn grow(&self, new_len: usize, f: impl FnMut() -> T) {
        if self.len() < new_len {
            let _guard = self.mutex.lock();
            self.do_grow(new_len, f);
        }
    }

    /// This function is technically lock-free despite the fact that `self.mutex` is
    /// used, since it only `try_lock` the mutex.
    pub(crate) fn try_grow(&self, new_len: usize, f: impl FnMut() -> T) -> Result<(), ()> {
        if self.len() < new_len {
            if let Some(_guard) = self.mutex.try_lock() {
                self.do_grow(new_len, f);
                Ok(())
            } else {
                Err(())
            }
        } else {
            Ok(())
        }
    }

    fn do_grow(&self, new_len: usize, f: impl FnMut() -> T) {
        let slice = self.as_slice();
        let slice_ref = &*slice;

        let old_len = slice_ref.len();
        if old_len >= new_len {
            return;
        }

        struct Initializer<'a, T, F>(Iter<'a, T>, usize, F);

        impl<T: Clone, F: FnMut() -> T> Iterator for Initializer<'_, T, F> {
            type Item = T;

            fn next(&mut self) -> Option<T> {
                if let Some(val) = self.0.next() {
                    Some(val.clone())
                } else if self.1 != 0 {
                    self.1 -= 1;
                    Some(self.2())
                } else {
                    None
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = self.0.len() + self.1;

                (len, Some(len))
            }
        }

        impl<T: Clone, F: FnMut() -> T> ExactSizeIterator for Initializer<'_, T, F> {}

        let arc =
            ThinArc::from_header_and_iter((), Initializer(slice_ref.iter(), new_len - old_len, f));

        let _old = self.array.swap(Some(arc));

        #[cfg(debug_assertions)]
        debug_assert!(slice.is_same_arc(_old.as_ref()));
    }
}

/// Slice is just a temporary borrow of the object.
pub(crate) struct Slice<'a, T>(Guard<Option<ThinArc<(), T>>>, PhantomData<&'a Arcs<T>>);

impl<T> Slice<'_, T> {
    #[cfg(debug_assertions)]
    fn is_same_arc(&self, other: Option<&ThinArc<(), T>>) -> bool {
        let this = self.0.as_ref();
        if this.is_none() && other.is_none() {
            return true;
        }

        let this = if let Some(this) = this {
            this
        } else {
            return false;
        };

        let other = if let Some(other) = other {
            other
        } else {
            return false;
        };

        this.heap_ptr() == other.heap_ptr()
    }
}

impl<T> Deref for Slice<'_, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .map(ThinArc::deref)
            .map(|header_slice| &header_slice.slice)
            .unwrap_or(&[])
    }
}

/// Thread sanitizer produces false positive in this test.
///
/// This has been discussed in
/// [this issue](https://github.com/vorner/arc-swap/issues/71)
/// and the failure can only be reproduced on x86-64-unknown-linux-gnu.
/// It cannot be reproduced on MacOS.
///
/// Since crate arc-swap is a cross platform crate with no assembly used
/// or any x86 specific feature, this can be some bugs in the allocator
/// or the thread sanitizer.
#[cfg(not(feature = "thread-sanitizer"))]
#[cfg(test)]
mod tests {
    use super::Arcs;

    use parking_lot::Mutex;
    use std::sync::Arc;

    use rayon::prelude::*;

    #[test]
    fn test() {
        let bag: Arc<Arcs<Arc<Mutex<u32>>>> = Arc::new(Arcs::new());
        assert_eq!(bag.len(), 0);
        assert!(bag.is_empty());

        {
            let slice = bag.as_slice();
            assert!(slice.is_empty());
            assert_eq!(slice.len(), 0);
        }

        bag.grow(10, Arc::default);
        {
            let slice = bag.as_slice();
            assert!(!slice.is_empty());

            for (i, arc) in slice.iter().enumerate() {
                *arc.lock() = i as u32;
            }
        }

        let bag_cloned = bag.clone();
        (0..u8::MAX).into_par_iter().for_each(move |_i| {
            bag_cloned.grow(bag_cloned.len() + 32, Arc::default);
        });

        {
            let slice = bag.as_slice();
            assert!(!slice.is_empty());

            for (i, arc) in slice.iter().take(10).enumerate() {
                *arc.lock() = i as u32;
            }
        }
    }
}
