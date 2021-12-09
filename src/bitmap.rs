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
        let bits = usize::BITS as usize;

        let chunk = &self.0[index / bits];
        let mask = !(1 << (index % bits));

        let mut value = chunk.load(Relaxed);

        loop {
            match compare_exchange(chunk, value, value & mask) {
                Ok(_) => break,
                Err(new_value) => value = new_value,
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn is_all_zero(&mut self) -> bool {
        for each in self.0.iter_mut() {
            if *each.get_mut() != 0 {
                return false;
            }
        }

        true
    }

    #[allow(unused)]
    pub(crate) fn is_all_one(&self) -> bool {
        for each in self.0.iter() {
            if each.load(Relaxed) != usize::MAX {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::BitMap;

    use parking_lot::Mutex;
    use std::sync::Arc;

    use bitvec::prelude::*;

    use std::thread::sleep;
    use std::time::Duration;

    use rayon::prelude::*;

    const LEN: usize = 512;

    #[test]
    fn test() {
        let bits = usize::BITS as usize;

        let mut bitvec = BitVec::<Lsb0, usize>::with_capacity(LEN * bits);
        bitvec.resize(LEN * bits, false);

        assert_eq!(bitvec.len(), LEN * bits);
        assert_eq!(bitvec.count_ones(), 0);

        let arc = Arc::new((
            BitMap::<LEN>::new(),
            Mutex::new(bitvec.into_boxed_bitslice()),
        ));

        let max_index = (LEN * bits) as usize;

        let arc_cloned = arc.clone();
        (0..(LEN * bits)).into_par_iter().for_each(|_| {
            let index = arc_cloned.0.allocate().unwrap();
            assert!(index <= max_index);
            assert!(arc_cloned.0.load(index as u32));
            assert!(!arc_cloned.1.lock().get_mut(index).unwrap().replace(true));
        });

        let bitmap = &arc.0;
        let bitvec = arc.1.lock();

        assert_eq!(bitvec.count_zeros(), 0);

        assert!(bitmap.is_all_one());

        assert!(bitmap.allocate().is_none());

        for i in 0..(LEN * bits) {
            assert!(bitmap.load(i as u32));
            bitmap.deallocate(i);

            assert!(!bitmap.load(i as u32));

            let index = bitmap.allocate().unwrap();
            assert_eq!(index, i);
            assert!(bitmap.load(i as u32));
        }
    }

    #[test]
    fn realworld_test() {
        let bits = usize::BITS as usize;

        let mut bitvec = BitVec::<Lsb0, usize>::with_capacity(LEN * bits);
        bitvec.resize(LEN * bits, false);

        assert_eq!(bitvec.len(), LEN * bits);
        assert_eq!(bitvec.count_ones(), 0);

        let arc = Arc::new((
            BitMap::<LEN>::new(),
            Mutex::new(bitvec.into_boxed_bitslice()),
        ));

        let arc_cloned = arc.clone();
        (0..(LEN * bits * 2)).into_par_iter().for_each(|_| {
            let index = loop {
                match arc_cloned.0.allocate() {
                    Some(index) => break index,
                    None => (),
                }
            };
            assert!(arc_cloned.0.load(index as u32));
            assert!(!arc_cloned.1.lock().get_mut(index).unwrap().replace(true));

            sleep(Duration::from_micros(1));

            let mut guard = arc_cloned.1.lock();
            arc_cloned.0.deallocate(index);
            assert!(guard.get_mut(index).unwrap().replace(false));
        });
    }
}
