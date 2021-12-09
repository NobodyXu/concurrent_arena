#!/bin/bash -ex

cd $(dirname `realpath $0`)

cargo test
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test

RUSTFLAGS='-Zsanitizer=thread' RUST_TEST_THREADS=1 cargo +nightly test

exec cargo +nightly miri test
