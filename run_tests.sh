#!/bin/bash -ex

cd $(dirname `realpath $0`)

cargo test
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test

export RUSTFLAGS='-Zsanitizer=thread'
export RUST_TEST_THREADS=1
exec cargo +nightly test
