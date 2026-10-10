# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.4.0

- `IoError::UniqueIdTaken` and `IoError::PermissionDenied` return on every platform and carry the backend's `PlatformError`, so callers match them without `cfg`; CoreMIDI `kMIDIIDNotUnique` lifts to `UniqueIdTaken`; CoreMIDI `kMIDINotPermitted`, ALSA `EACCES`/`EPERM` and Web `NotAllowedError`/`SecurityError` lift to `PermissionDenied`; `Unsupported` and `PortDisconnected` take an `Option<PlatformError>`, and Web `NotSupportedError` and `InvalidStateError` lift to them (https://github.com/molenick/midi-io/pull/22)
- The ALSA destination subscription test asks the sequencer for subscribers instead of reading `/proc/asound/seq/clients`, which `container` 1.5.0 hides (https://github.com/molenick/midi-io/pull/23)
- CI and `verify.sh` run clippy with `--no-default-features`; the e2e tests require the `io` feature; `ORPHAN_PREFIX_BYTES` builds only where it is used (https://github.com/molenick/midi-io/pull/24)

## 0.3.0

- `PlatformError` now carries the backend's own error (`AlsaError`, `CoreMidiError`, `WebError`) instead of a bare code; `Encode` moves to `IoError`; `ThreadInit` becomes `ThreadSpawn(io::Error)`; `IoError::Web`, `UniqueIdTaken` and `PermissionDenied` are removed, so platform errors always arrive as `Platform` with the backend's own text; `Error` no longer derives `Clone`, `PartialEq`, `Eq` (https://github.com/molenick/midi-io/pull/20)
- Web synth example: port lists, MIDI log, browser support legend (https://github.com/molenick/midi-io/pull/19)

## 0.2.1

- Actually send to ALSA destinations (https://github.com/molenick/midi-io/pull/17)

## 0.2.0

- Export `Instant`, so the type of `Timed::timestamp` can be named on every platform
- Add `Client::create_virtual_destination_with_id`, so a virtual destination keeps its unique ID across launches (https://github.com/molenick/midi-io/pull/15)
- Add `PortId::to_bits`, so a port can be stored and matched in a later session (https://github.com/molenick/midi-io/pull/14)
- Send MIDI on the web (wasm32) backend: destinations connect (eager `open()`) and send (https://github.com/molenick/midi-io/pull/13)
- Warn Firefox users in the web synth example (https://github.com/molenick/midi-io/pull/12)
- Bump Pages actions to Node 24 versions (https://github.com/molenick/midi-io/pull/11)
- Initial web MIDI backend (https://github.com/molenick/midi-io/pull/10)
- Reshape `PortId` into an opaque u64 handle (https://github.com/molenick/midi-io/pull/9)

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
