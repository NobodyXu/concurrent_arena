#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUST_TEST_THREADS=1

rep=$(seq 1 10)

for _ in $rep; do
    cargo test -- --nocapture
done

for _ in $rep; do
    RUSTFLAGS='-Zsanitizer=address' cargo +nightly test -- --nocapture
done
    
for _ in $rep; do
    RUSTFLAGS='-Zsanitizer=thread' cargo +nightly test \
        -Z build-std --target --target $(uname -m)-unknown-linux-gnu -- --nocapture
done

export MIRIFLAGS="-Zmiri-disable-isolation"
for _ in $rep; do
    cargo +nightly miri test -- --nocapture
done
