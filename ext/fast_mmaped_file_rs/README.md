# Testing

## Running Tests

Use [cargo nextest](https://nexte.st/) to execute the Rust unit tests.

```sh
$ cargo nextest run
```

## Why not use 'cargo test'?

We need to embed Ruby into the test binary to access Ruby types. This requires
us to run `magnus::embed::init()` no more than once before calling Ruby.
See [the magnus docs](https://docs.rs/magnus/latest/magnus/embed/fn.init.html)
for more details.

If we try to create separate `#[test]` functions that call `init()` these will
conflict, as Cargo runs tests in parallel using a single process with separate
threads. Running `cargo test` will result in errors like:

```
---- file_info::test::with_ruby stdout ----
thread 'file_info::test::with_ruby' panicked at 'Ruby already initialized'
```

The simplest workaround for this is to avoid using `cargo test` to run unit
tests. [nextest](https://nexte.st/) is an alternate test harness that runs each
test as its own process, enabling each test to intitialize Ruby without
conflict.

## 'symbol not found' errors when running tests

If you see errors like the following when running tests:

```
Caused by:
  for `fast_mmaped_file_rs`, command `/Users/myuser/prometheus-client-mmap/ext/fast_mmaped_file_rs/target/debug/deps/fast_mmaped_file_rs-c81ccc96a6484e04 --list --format terse` exited with signal 6 (SIGABRT)
--- stdout:

--- stderr:
dyld[17861]: symbol not found in flat namespace '_rb_cArray'
```

Clearing the build cache will resolve the problem.

```sh
$ cargo clean
```

This is probably due to separate features being used with `magnus` in
development builds.
