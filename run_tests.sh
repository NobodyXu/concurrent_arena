#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUST_TEST_THREADS=1

rep=$(seq 1 10)

for _ in $rep; do
    cargo test -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=address'
for _ in $rep; do
    cargo +nightly test -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=thread' 
for _ in $rep; do
    cargo +nightly test \
        -Z build-std \
        --target --target $(uname -m)-unknown-linux-gnu \
        --features thread-sanitizer \
        -- --nocapture
done

export MIRIFLAGS="-Zmiri-disable-isolation"
for _ in $rep; do
    cargo +nightly miri test -- --nocapture
done
