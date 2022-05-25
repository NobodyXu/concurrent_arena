#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUST_TEST_THREADS=1

rep=$(seq 1 10)

for _ in $rep; do
    cargo test $@ -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=address'
export RUSTDOCFLAGS="$RUSTFLAGS"
for _ in $rep; do
    cargo +nightly test $@ -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=thread' 
export RUSTDOCFLAGS="$RUSTFLAGS"
for _ in $rep; do
    cargo +nightly test $@ \
        -Z build-std \
        --target $(uname -m)-unknown-linux-gnu \
        --features thread-sanitizer \
        -- --nocapture
done

#export MIRIFLAGS="-Zmiri-disable-isolation"
#exec cargo +nightly miri test -- --nocapture
