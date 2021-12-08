#!/bin/bash -ex

cd $(dirname `realpath $0`)

cargo test
RUSTFLAGS='-Zsanitizer=address' cargo +nightly test

export RUSTFLAGS='-Zsanitizer=thread'
exec cargo +nightly test
