#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUST_TEST_THREADS=1

cargo test -- --nocapture
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test -- --nocapture

export RUSTFLAGS='-Zsanitizer=thread'
exec cargo +nightly test \
    -Z build-std --target --target $(uname -m)-unknown-linux-gnu -- --nocapture

#export MIRIFLAGS="-Zmiri-disable-isolation"
#exec cargo +nightly miri test -- --nocapture
