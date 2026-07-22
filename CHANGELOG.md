# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- Send MIDI on the web (wasm32) backend: destinations connect (eager `open()`) and send (https://github.com/molenick/midi-io/pull/13)
- Bump Pages actions to Node 24 versions (https://github.com/molenick/midi-io/pull/11)
- Initial web MIDI backend (https://github.com/molenick/midi-io/pull/10)
- Reshape PortId into an opaque u64 handle (https://github.com/molenick/midi-io/pull/9)

## 0.1.2

- Use mach primitives for host time on Apple platforms https://github.com/molenick/midi-io/pull/7
- Add crates.io version badge to README https://github.com/molenick/midi-io/pull/6
- Update deps https://github.com/molenick/midi-io/pull/5

## 0.1.1

- Disable iOS simulator test runs on ci (too slow) https://github.com/molenick/midi-io/pull/3
- Fixed broken repo link ink Cargo.toml https://github.com/molenick/midi-io/pull/2
- Disable simulated ALSA integration tests on ci (lack of support) https://github.com/molenick/midi-io/pull/1

## 0.1.0

Initial release.

- Strictly-typed MIDI 1.0 message model (`MidiMessage`, `SysEx`, `RawMidiMessage`)
  with parse-don't-validate construction and a cross-platform `decode` function.
- Async `Client` for live MIDI on CoreMIDI (macOS, iOS) and the ALSA sequencer
  (Linux): source/destination enumeration, hotplug change streams, connections
  with separate message/SysEx/error streams, and virtual sources/destinations.
- Bounded inbound streams with coalesced overflow reporting; timestamps as
  `std::time::Instant` from backend packet/queue time.
- `io` (default) and `tracing` cargo features; codec-only use via
  `default-features = false`.
