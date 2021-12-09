#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUST_TEST_THREADS=1

cargo test -- --nocapture
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test -- --nocapture

RUSTFLAGS='-Zsanitizer=thread' cargo +nightly test \
    -Z build-std --target --target x86_64-unknown-linux-gnu -- --nocapture

exec cargo +nightly miri test -- --nocapture
