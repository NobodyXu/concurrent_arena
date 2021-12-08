use super::thread_id::get_thread_id;

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;

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

    pub(crate) fn load(&self, index: u32) -> bool {
        let bits = usize::BITS;
        let mask = 1 << (index % bits);
        let offset = (index / bits) as usize;

        (self.0[offset].load(Relaxed) & mask) != 0
    }

    pub(crate) fn allocate(&self) -> Option<usize> {
        let mut pos = get_thread_id() % BITARRAY_LEN;

        let slice1_iter = self.0[pos..].iter();
        let slice2_iter = self.0[..pos].iter();

        for chunk in slice1_iter.chain(slice2_iter) {
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
