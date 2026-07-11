# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

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
