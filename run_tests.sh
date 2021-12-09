#!/bin/bash -ex

cd $(dirname `realpath $0`)

cargo test -- --nocapture
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test -- --nocapture

RUSTFLAGS='-Zsanitizer=thread' RUST_TEST_THREADS=1 cargo +nightly test -- --nocapture

exec cargo +nightly miri test -- --nocapture
