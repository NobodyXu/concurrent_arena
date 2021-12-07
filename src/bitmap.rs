use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;

use parking_lot::lock_api::GetThreadId;
use parking_lot::RawThreadId;

use array_init::array_init;

fn compare_exchange(atomic: &AtomicUsize, curr: usize, new: usize) -> Result<(), usize> {
    atomic
        .compare_exchange_weak(curr, new, Relaxed, Relaxed)
        .map(|_| ())
}

/// * `BITARRAY_LEN` - the number of AtomicUsize
#[derive(Debug)]
pub(crate) struct BitMap<const BITARRAY_LEN: usize>([AtomicUsize; BITARRAY_LEN]);

impl<const BITARRAY_LEN: usize> BitMap<BITARRAY_LEN> {
    pub(crate) fn new() -> Self {
        Self(array_init(|_| AtomicUsize::new(0)))
    }

    pub(crate) fn allocate(&self) -> Option<usize> {
        let mut pos = RawThreadId::INIT.nonzero_thread_id().get() % BITARRAY_LEN;

        for _ in 0..BITARRAY_LEN {
            let chunk = &self.0[pos];
            let mut value = chunk.load(Relaxed);

            loop {
                if value == usize::MAX {
                    break;
                }

                let bits = usize::BITS as usize;

                for i in 0..bits {
                    let mask = 1 << i;
                    if (value & mask) != 0 {
                        continue;
                    }

                    match compare_exchange(chunk, value, value | mask) {
                        Ok(_) => {
                            return Some(pos * bits + i);
                        }
                        Err(new_value) => {
                            value = new_value;
                            // try again
                            break;
                        }
                    }
                }
            }

            pos = (pos + 1) % BITARRAY_LEN;
        }

        None
    }

    pub(crate) fn deallocate(&self, index: usize) {
        let chunk = &self.0[index];
        let mut value = chunk.load(Relaxed);
        let mask = !(1 << index);

        loop {
            match compare_exchange(chunk, value, value & mask) {
                Ok(_) => break,
                Err(new_value) => value = new_value,
            }
        }
    }
}
