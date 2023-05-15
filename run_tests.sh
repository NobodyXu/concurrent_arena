#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUST_TEST_THREADS=1

rep=$(seq 1 10)

for _ in $rep; do
    # shellcheck disable=SC2068
    cargo test $@ -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=address'
export RUSTDOCFLAGS="$RUSTFLAGS"
for _ in $rep; do
    # shellcheck disable=SC2068
    cargo +nightly test $@ -- --nocapture
done

export RUSTFLAGS='-Zsanitizer=thread' 
export RUSTDOCFLAGS="$RUSTFLAGS"

target="$(rustc -vV | grep host | cut -d : -f 2 | tr -d '[:space:]')"
for _ in $rep; do
    # shellcheck disable=SC2068
    cargo +nightly test $@ \
        -Z build-std \
        --target "$target" \
        --features thread-sanitizer \
        -- --nocapture
done

#export MIRIFLAGS="-Zmiri-disable-isolation"
#exec cargo +nightly miri \
#    nextest run \
#    -Z build-std \
#    --target "$target" \
#    --release
