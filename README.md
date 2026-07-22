# midi-io

[![crates.io](https://img.shields.io/crates/v/midi-io.svg)](https://crates.io/crates/midi-io)

A portable library for encoding, sending, decoding and streaming MIDI

## Features
- Send and receive strongly-typed MIDI 1 messages
- Async-first API with any runtime
- Receive messages as individual streams: MIDI, SysEx, backend notifications, errors
- Connection management: receive and react to connection/disconnection event notifications

## Supported Platforms

| OS | Platform | Library |
|----------|----------|----------|
| macOS  10.15+| CoreMIDI | [coremidi](https://crates.io/crates/coremidi) |
| iOS 15+ | CoreMIDI | [coremidi](https://crates.io/crates/coremidi) |
| Linux 3.x+ | ALSA sequencer | [alsa](https://crates.io/crates/alsa) |
| Web (wasm32) | Web MIDI | Chrome, Firefox (limited) |

On wasm32, only receiving is supported; sending and virtual ports return `Unsupported`, and `Client` is not `Send`. Web MIDI bindings are unstable, so builds require `RUSTFLAGS=--cfg=web_sys_unstable_apis` (set for the target in `.cargo/config.toml`).

**[Live demo](https://molenick.github.io/midi-io/):** a browser synth driven by the Web MIDI backend. Needs a web MIDI compatible browser and a connected MIDI controller.

If you need a platform that isn't yet supported, check out [midir](https://github.com/Boddlnagg/midir).

## Example

Connect to the first MIDI source and print every message as it arrives:

```rust,no_run
use midi_io::Client;

#[tokio::main]
async fn main() -> Result<(), midi_io::Error> {
    let client = Client::new("my-app").await?;
    let sources = client.sources().await?;
    let source = sources.first().expect("no MIDI sources found");
    let mut messages = client.connect_source(source).await?.into_messages();

    while let Some(timed) = messages.recv().await {
        println!("{}", timed.payload);
    }
    Ok(())
}
```

See [examples](examples/) for more.

## Installation

```sh
cargo add midi-io
```

For codec-only use, disable default features:
`midi-io = { version = "0.1", default-features = false }`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
