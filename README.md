# asyncsync-rs

[![Crates.io](https://img.shields.io/crates/v/asyncsync)](https://crates.io/crates/asyncsync)
[![Docs.rs](https://img.shields.io/docsrs/asyncsync/latest)](https://docs.rs/asyncsync)

asyncsync aims to provide the commonly required synchronization primitives for asynchronous Rust in an runtime-agnostic and performant way.

## Usage

Add `asyncsync` to your dependencies:

```
asyncsync = "0.1.0"
```

See [docs.rs](https://docs.rs/asyncsync) for details about specific types.

## Features

`std`: Enables usage of std. This is currently required for the default `Send` primitives. Enabled by default.  
`local`: Enables the optional [local module](https://docs.rs/asyncsync/latest/asyncsync/local), providing `!Send` primitives for a single-threaded context.

## License

Licensed under either The [Apache License, Version 2.0](https://github.com/MrGunflame/asyncsync-rs/blob/master/LICENSE-APACHE) or [MIT license](https://github.com/MrGunflame/asyncsync-rs/blob/master/LICENSE-MIT) at your option.
