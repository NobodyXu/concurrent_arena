# ConcurrentArena

[![Rust](https://github.com/NobodyXu/concurrent_arena/actions/workflows/rust.yml/badge.svg)](https://github.com/NobodyXu/concurrent_arena/actions/workflows/rust.yml)

[![crate.io downloads](https://img.shields.io/crates/d/concurrent_arena)](https://crates.io/crates/concurrent_arena)

[![crate.io version](https://img.shields.io/crates/v/concurrent_arena)](https://crates.io/crates/concurrent_arena)

[![docs](https://docs.rs/concurrent_arena/badge.svg)](https://docs.rs/concurrent_arena)

Concurrent arena that
 - Support concurrent inserted and removed;
 - Use a `u32` as key;
 - Returns `ArenaArc` to track the inserted object to avoid lifetime issues.

## How to run tests

```
./run_tests.sh
```
