use crate::midi::decode::decode;
use crate::midi::decode::expected_len;
use crate::midi::decode::sysex_decode_error;
use crate::midi::decode::Decoded;
use crate::midi::message::MidiMessage;
use crate::midi::sys_ex::SysEx;
use crate::midi::sys_ex::MAX_SYSEX_BYTES;
use crate::midi::sys_ex::ORPHAN_PREFIX_BYTES;
use crate::CodecError;
use crate::ParseError;

pub(crate) struct StreamParser {
    running_status: Option<u8>,
    system_common_status: Option<u8>,
    sysex_buf: Vec<u8>,
    in_sysex: bool,
    sysex_overflow: bool,
    pending: Vec<u8>,
    orphan_buf: Vec<u8>,
    orphan_len: usize,
}

impl Default for StreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamParser {
    pub fn new() -> Self {
        Self {
            running_status: None,
            system_common_status: None,
            sysex_buf: Vec::new(),
            in_sysex: false,
            sysex_overflow: false,
            pending: Vec::new(),
            orphan_buf: Vec::new(),
            orphan_len: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8], emit: &mut impl FnMut(DecodedEvent)) {
        for &b in bytes {
            if b >= 0xF8 {
                match decode(&[b]) {
                    Ok(Decoded::Message(msg)) => emit(DecodedEvent::Message(msg)),
                    Ok(Decoded::SysEx(_)) => {
                        unreachable!("a single realtime byte cannot decode to sysex")
                    }
                    Err(e) => emit(DecodedEvent::Error(e.into())),
                }
                continue;
            }

            if b == 0xF0 {
                self.flush_orphans(emit);
                if self.in_sysex {
                    self.abort_sysex(emit);
                }
                self.in_sysex = true;
                self.sysex_buf.clear();
                self.pending.clear();
                self.running_status = None;
                self.system_common_status = None;
                continue;
            }

            if self.in_sysex {
                if b == 0xF7 {
                    self.in_sysex = false;
                    if self.sysex_overflow {
                        self.sysex_overflow = false;
                    } else {
                        let body = std::mem::take(&mut self.sysex_buf);
                        let mut wire = Vec::with_capacity(body.len() + 2);
                        wire.push(0xF0);
                        wire.extend_from_slice(&body);
                        wire.push(0xF7);
                        match SysEx::try_from(wire.as_slice()) {
                            Ok(sysex) => emit(DecodedEvent::Sysex(sysex)),
                            Err(e) => emit(DecodedEvent::Error(sysex_decode_error(e, wire).into())),
                        }
                        self.sysex_buf.clear();
                    }
                } else if b >= 0x80 {
                    self.abort_sysex(emit);
                    if b < 0xF0 {
                        self.start_status(b);
                    } else {
                        self.handle_system_common(b, emit);
                    }
                } else if !self.sysex_overflow {
                    if self.sysex_buf.len() + 1 > MAX_SYSEX_BYTES {
                        emit(DecodedEvent::Error(
                            CodecError::SysexTooLong {
                                len: self.sysex_buf.len() + 1,
                                max: MAX_SYSEX_BYTES,
                            }
                            .into(),
                        ));
                        self.sysex_overflow = true;
                        self.sysex_buf.clear();
                    } else {
                        self.sysex_buf.push(b);
                    }
                }
                continue;
            }

            if b >= 0x80 {
                if b == 0xF7 && self.orphan_len > 0 {
                    if self.orphan_buf.len() < ORPHAN_PREFIX_BYTES {
                        self.orphan_buf.push(b);
                    }
                    self.orphan_len += 1;
                    self.flush_orphans(emit);
                    continue;
                }
                self.flush_orphans(emit);
                if b < 0xF0 {
                    self.system_common_status = None;
                    self.start_status(b);
                } else {
                    self.handle_system_common(b, emit);
                }
                continue;
            }

            if let Some(sc_status) = self.system_common_status {
                self.pending.push(b);
                let needed = expected_len(sc_status).map_or(0, |total| total - 1);
                if self.pending.len() == needed {
                    let mut packet = vec![sc_status];
                    packet.extend_from_slice(&self.pending);
                    self.pending.clear();
                    self.system_common_status = None;
                    match decode(&packet) {
                        Ok(Decoded::Message(msg)) => emit(DecodedEvent::Message(msg)),
                        Ok(Decoded::SysEx(sysex)) => emit(DecodedEvent::Sysex(sysex)),
                        Err(e) => emit(DecodedEvent::Error(e.into())),
                    }
                }
                continue;
            }

            if let Some(status) = self.running_status {
                self.pending.push(b);
                let needed = expected_len(status).map_or(0, |total| total - 1);
                if needed > 0 && self.pending.len() == needed {
                    let mut packet = vec![status];
                    packet.extend_from_slice(&self.pending);
                    self.pending.clear();
                    match decode(&packet) {
                        Ok(Decoded::Message(msg)) => emit(DecodedEvent::Message(msg)),
                        Ok(Decoded::SysEx(sysex)) => emit(DecodedEvent::Sysex(sysex)),
                        Err(e) => emit(DecodedEvent::Error(e.into())),
                    }
                }
            } else {
                if self.orphan_buf.len() < ORPHAN_PREFIX_BYTES {
                    self.orphan_buf.push(b);
                }
                self.orphan_len += 1;
            }
        }
    }

    fn flush_orphans(&mut self, emit: &mut impl FnMut(DecodedEvent)) {
        if self.orphan_len == 0 {
            return;
        }
        let len = std::mem::take(&mut self.orphan_len);
        let bytes = std::mem::take(&mut self.orphan_buf);
        emit(DecodedEvent::Error(
            CodecError::Parse {
                reason: ParseError::OrphanedData { len },
                bytes,
            }
            .into(),
        ));
    }

    fn start_status(&mut self, status: u8) {
        self.pending.clear();
        self.running_status = Some(status);
    }

    fn abort_sysex(&mut self, emit: &mut impl FnMut(DecodedEvent)) {
        self.in_sysex = false;
        if self.sysex_overflow {
            self.sysex_overflow = false;
            return;
        }
        let body = std::mem::take(&mut self.sysex_buf);
        let mut bytes = Vec::with_capacity(body.len() + 1);
        bytes.push(0xF0);
        bytes.extend_from_slice(&body);
        emit(DecodedEvent::Error(
            CodecError::Parse {
                reason: ParseError::UnterminatedSysex,
                bytes,
            }
            .into(),
        ));
    }

    fn handle_system_common(&mut self, b: u8, emit: &mut impl FnMut(DecodedEvent)) {
        self.running_status = None;
        self.system_common_status = None;
        self.pending.clear();
        if b == 0xF6 {
            emit(DecodedEvent::Message(MidiMessage::TuneRequest));
        } else if b <= 0xF3 {
            self.system_common_status = Some(b);
        } else if let Err(e) = decode(&[b]) {
            emit(DecodedEvent::Error(e.into()));
        }
    }
}

pub(crate) enum DecodedEvent {
    Message(MidiMessage),
    Sysex(SysEx),
    Error(crate::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::channel::Channel;
    use crate::midi::conformance;
    use crate::midi::data_byte::DataByte;
    use crate::midi::song_position::SongPosition;

    fn sp(n: u16) -> SongPosition {
        SongPosition::try_from(n).unwrap()
    }

    #[test]
    fn conformance_stream_parser_matches_corpus() {
        for (bytes, expected) in conformance::all() {
            let mut events = Vec::new();
            let mut parser = StreamParser::new();
            parser.push(&bytes, &mut |e| events.push(e));
            assert_eq!(events.len(), 1, "one event for {bytes:02X?}");
            match events.into_iter().next().unwrap() {
                DecodedEvent::Message(m) => assert_eq!(m, expected, "stream parser {bytes:02X?}"),
                DecodedEvent::Sysex(_) => panic!("expected message for {bytes:02X?}, got sysex"),
                DecodedEvent::Error(e) => {
                    panic!("expected message for {bytes:02X?}, got error {e:?}")
                }
            }
        }
    }

    #[test]
    fn conformance_sysex_decode_and_stream_parser_agree() {
        let body = match decode(&conformance::SYSEX_FRAME) {
            Ok(Decoded::SysEx(s)) => s,
            other => panic!("decode sysex: {other:?}"),
        };
        assert_eq!(body.bytes(), &conformance::SYSEX_BODY);

        let mut got = Vec::new();
        let mut parser = StreamParser::new();
        parser.push(&conformance::SYSEX_FRAME, &mut |e| {
            if let DecodedEvent::Sysex(s) = e {
                got.push(s);
            }
        });
        assert_eq!(
            got,
            vec![SysEx::try_from(conformance::SYSEX_FRAME.as_slice()).unwrap()]
        );
    }

    #[test]
    fn stream_parser_running_status() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0x90, 60, 100, 62, 80], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(
            msgs,
            vec![
                MidiMessage::NoteOn {
                    channel: Channel::Ch1,
                    key: DataByte::try_from(60).unwrap(),
                    velocity: DataByte::try_from(100).unwrap()
                },
                MidiMessage::NoteOn {
                    channel: Channel::Ch1,
                    key: DataByte::try_from(62).unwrap(),
                    velocity: DataByte::try_from(80).unwrap()
                },
            ]
        );
    }

    #[test]
    fn stream_parser_sysex_split_across_packets() {
        let mut parser = StreamParser::new();
        let mut sys = Vec::new();
        parser.push(&[0xF0, 0x41], &mut |m| {
            if let DecodedEvent::Sysex(s) = m {
                sys.push(s);
            }
        });
        assert!(sys.is_empty());
        parser.push(&[0x10, 0xF7], &mut |m| {
            if let DecodedEvent::Sysex(s) = m {
                sys.push(s);
            }
        });
        assert_eq!(sys, vec![SysEx::new(&[0x41, 0x10]).unwrap()]);
    }

    #[test]
    fn stream_parser_realtime_interleaved() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0x90, 0xF8, 60, 100], &mut |m| {
            if let DecodedEvent::Message(MidiMessage::TimingClock) = m {
                msgs.push(());
            }
        });
        assert!(!msgs.is_empty());
    }

    #[test]
    fn stream_parser_undefined_realtime_emits_error() {
        for status in [0xF9u8, 0xFD] {
            let mut parser = StreamParser::new();
            let mut events = Vec::new();
            parser.push(&[status], &mut |m| events.push(m));
            assert_eq!(events.len(), 1);
            assert!(matches!(
                &events[0],
                DecodedEvent::Error(crate::Error::Codec(CodecError::Parse {
                    reason: ParseError::UnrecognizedStatus,
                    ..
                }))
            ));
        }
    }

    #[test]
    fn stream_parser_realtime_f9_does_not_consume_following_byte() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0x90, 60, 100, 0xF9, 62, 80], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(
            msgs,
            vec![
                MidiMessage::NoteOn {
                    channel: Channel::Ch1,
                    key: DataByte::try_from(60).unwrap(),
                    velocity: DataByte::try_from(100).unwrap()
                },
                MidiMessage::NoteOn {
                    channel: Channel::Ch1,
                    key: DataByte::try_from(62).unwrap(),
                    velocity: DataByte::try_from(80).unwrap()
                },
            ]
        );
    }

    #[test]
    fn stream_parser_system_common_cancels_running_status() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0x90, 60, 100], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs.len(), 1);
        parser.push(&[0xF2, 0x00, 0x00], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1], MidiMessage::SongPositionPointer(sp(0)));
        parser.push(&[62, 80], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(
            msgs.len(),
            2,
            "data bytes after system common must not emit"
        );
    }

    #[test]
    fn stream_parser_system_common_mtc_two_byte() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0xF1, 0x35], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(
            msgs,
            vec![MidiMessage::MtcQuarterFrame(
                DataByte::try_from(0x35).unwrap()
            )]
        );
    }

    #[test]
    fn stream_parser_system_common_song_position_pointer() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0xF2, 0x00, 0x02], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs, vec![MidiMessage::SongPositionPointer(sp(256))]);
    }

    #[test]
    fn stream_parser_system_common_tune_request_immediate() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0xF6], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs, vec![MidiMessage::TuneRequest]);
    }

    #[test]
    fn stream_parser_system_common_interleaved_with_channel() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0x90, 60, 100, 0xF2, 0x00, 0x02], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs.len(), 2);
        assert_eq!(
            msgs[0],
            MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }
        );
        assert_eq!(msgs[1], MidiMessage::SongPositionPointer(sp(256)));
        parser.push(&[62, 80], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn stream_parser_realtime_during_system_common() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        parser.push(&[0xF2, 0x00, 0xF8, 0x02], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(msgs.len(), 2);
        assert!(msgs.contains(&MidiMessage::TimingClock));
        assert!(msgs.contains(&MidiMessage::SongPositionPointer(sp(256))));
    }

    #[test]
    fn stream_parser_orphan_data_bytes_coalesce_into_one_error_on_resync() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        let mut errors = Vec::new();
        parser.push(&[60, 100, 62], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => {}
        });
        assert!(msgs.is_empty());
        assert!(errors.is_empty(), "orphan run is pending until resync");
        parser.push(&[0x90, 60, 100], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => {}
        });
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 3 },
                bytes,
            }) if bytes == &vec![60, 100, 62]
        ));
        assert_eq!(
            msgs,
            vec![MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }]
        );
    }

    #[test]
    fn stream_parser_orphaned_sysex_tail_coalesces_into_one_error() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        let mut errors = Vec::new();
        parser.push(&[0x01, 0x02, 0x03, 0xF7], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => panic!("unexpected sysex"),
        });
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 4 },
                bytes,
            }) if bytes == &vec![0x01, 0x02, 0x03, 0xF7]
        ));
        parser.push(&[0x90, 60, 100], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => panic!("unexpected sysex"),
        });
        assert_eq!(errors.len(), 1);
        assert_eq!(
            msgs,
            vec![MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }]
        );
    }

    #[test]
    fn stream_parser_orphan_error_caps_stored_bytes() {
        let mut parser = StreamParser::new();
        let mut errors = Vec::new();
        parser.push(&[0x01u8; 200], &mut |m| {
            if let DecodedEvent::Error(e) = m {
                errors.push(e);
            }
        });
        assert!(errors.is_empty());
        parser.push(&[0x90, 60, 100], &mut |m| {
            if let DecodedEvent::Error(e) = m {
                errors.push(e);
            }
        });
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 200 },
                bytes,
            }) if bytes == &vec![0x01; ORPHAN_PREFIX_BYTES]
        ));
    }

    #[test]
    fn stream_parser_realtime_does_not_flush_orphan_run() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        let mut errors = Vec::new();
        parser.push(&[60, 0xF8, 100, 0x90, 62, 80], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => panic!("unexpected sysex"),
        });
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 2 },
                bytes,
            }) if bytes == &vec![60, 100]
        ));
        assert_eq!(
            msgs,
            vec![
                MidiMessage::TimingClock,
                MidiMessage::NoteOn {
                    channel: Channel::Ch1,
                    key: DataByte::try_from(62).unwrap(),
                    velocity: DataByte::try_from(80).unwrap()
                }
            ]
        );
    }

    #[test]
    fn stream_parser_standalone_sysex_end_emits_error() {
        let mut parser = StreamParser::new();
        let mut errors = Vec::new();
        parser.push(&[0xF7], &mut |m| {
            if let DecodedEvent::Error(e) = m {
                errors.push(e);
            }
        });
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn stream_parser_sysex_aborts_pending_system_common() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        let mut sys = Vec::new();
        let mut errors = Vec::new();
        parser.push(
            &[0xF2, 0x00, 0xF0, 0x01, 0xF7, 0x02, 0x03],
            &mut |m| match m {
                DecodedEvent::Message(msg) => msgs.push(msg),
                DecodedEvent::Sysex(s) => sys.push(s),
                DecodedEvent::Error(e) => errors.push(e),
            },
        );
        assert!(
            msgs.is_empty(),
            "post-sysex data bytes must not complete the aborted system common message"
        );
        assert_eq!(sys, vec![SysEx::new(&[0x01]).unwrap()]);
        assert!(errors.is_empty(), "orphan run is pending until resync");
        parser.push(&[0xF6], &mut |m| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Sysex(s) => sys.push(s),
            DecodedEvent::Error(e) => errors.push(e),
        });
        assert_eq!(errors.len(), 1, "orphan data bytes coalesce into one error");
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::OrphanedData { len: 2 },
                bytes,
            }) if bytes == &vec![0x02, 0x03]
        ));
        assert_eq!(msgs, vec![MidiMessage::TuneRequest]);
    }

    #[test]
    fn stream_parser_interrupted_sysex_emits_unterminated_error() {
        let mut parser = StreamParser::new();
        let mut msgs = Vec::new();
        let mut errors = Vec::new();
        let mut sink = |m: DecodedEvent| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Sysex(_) => panic!("unexpected sysex"),
        };
        parser.push(&[0xF0, 0x41], &mut sink);
        parser.push(&[0x90], &mut sink);
        parser.push(&[60, 100], &mut sink);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::UnterminatedSysex,
                bytes,
            }) if bytes == &vec![0xF0, 0x41]
        ));
        assert_eq!(
            msgs,
            vec![MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }]
        );
    }

    #[test]
    fn stream_parser_sysex_restart_emits_unterminated_error() {
        let mut parser = StreamParser::new();
        let mut sys = Vec::new();
        let mut errors = Vec::new();
        parser.push(&[0xF0, 0x01, 0xF0, 0x02, 0xF7], &mut |m| match m {
            DecodedEvent::Sysex(s) => sys.push(s),
            DecodedEvent::Error(e) => errors.push(e),
            DecodedEvent::Message(_) => panic!("unexpected message"),
        });
        assert_eq!(sys, vec![SysEx::new(&[0x02]).unwrap()]);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            crate::Error::Codec(CodecError::Parse {
                reason: ParseError::UnterminatedSysex,
                bytes,
            }) if bytes == &vec![0xF0, 0x01]
        ));
    }

    #[test]
    fn stream_parser_oversized_sysex_emits_error_and_recovers() {
        let mut parser = StreamParser::new();
        let mut errored = false;
        let mut msgs = Vec::new();

        let mut overflow = vec![0xF0u8];
        overflow.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 1));
        parser.push(&overflow, &mut |m| match m {
            DecodedEvent::Sysex(_) => panic!("unexpected sysex"),
            DecodedEvent::Error(crate::Error::Codec(CodecError::SysexTooLong { max, .. })) => {
                assert_eq!(max, MAX_SYSEX_BYTES);
                errored = true;
            }
            DecodedEvent::Error(e) => panic!("unexpected error: {e:?}"),
            DecodedEvent::Message(_) => panic!("unexpected message"),
        });
        assert!(errored, "oversized sysex must emit SysexTooLong");

        parser.push(&[0x90, 60, 100], &mut |m| {
            if let DecodedEvent::Message(msg) = m {
                msgs.push(msg);
            }
        });
        assert_eq!(
            msgs,
            vec![MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }],
            "parser must recover after an oversized sysex"
        );
    }

    #[test]
    fn stream_parser_oversized_sysex_discards_remainder_with_single_error() {
        let mut parser = StreamParser::new();
        let mut errors = 0;
        let mut msgs = Vec::new();
        let mut sys = Vec::new();

        let mut bytes = vec![0xF0u8];
        bytes.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 50));
        bytes.push(0xF7);
        bytes.extend([0x90, 60, 100]);
        let mut sink = |m: DecodedEvent| match m {
            DecodedEvent::Message(msg) => msgs.push(msg),
            DecodedEvent::Sysex(s) => sys.push(s),
            DecodedEvent::Error(crate::Error::Codec(CodecError::SysexTooLong { max, .. })) => {
                assert_eq!(max, MAX_SYSEX_BYTES);
                errors += 1;
            }
            DecodedEvent::Error(e) => panic!("unexpected error: {e:?}"),
        };
        parser.push(&bytes[..MAX_SYSEX_BYTES], &mut sink);
        parser.push(&bytes[MAX_SYSEX_BYTES..], &mut sink);

        assert_eq!(errors, 1, "exactly one error for the whole oversized sysex");
        assert!(sys.is_empty());
        assert_eq!(
            msgs,
            vec![MidiMessage::NoteOn {
                channel: Channel::Ch1,
                key: DataByte::try_from(60).unwrap(),
                velocity: DataByte::try_from(100).unwrap()
            }]
        );
    }

    #[test]
    fn stream_parser_new_sysex_during_oversized_discard_parses_cleanly() {
        let mut parser = StreamParser::new();
        let mut errors = 0;
        let mut sys = Vec::new();

        let mut bytes = vec![0xF0u8];
        bytes.extend(std::iter::repeat(0x01).take(MAX_SYSEX_BYTES + 50));
        bytes.extend([0xF0, 0x41, 0x10, 0xF7]);
        parser.push(&bytes, &mut |m| match m {
            DecodedEvent::Sysex(s) => sys.push(s),
            DecodedEvent::Error(crate::Error::Codec(CodecError::SysexTooLong { .. })) => {
                errors += 1;
            }
            DecodedEvent::Error(e) => panic!("unexpected error: {e:?}"),
            DecodedEvent::Message(_) => panic!("unexpected message"),
        });

        assert_eq!(errors, 1);
        assert_eq!(sys, vec![SysEx::new(&[0x41, 0x10]).unwrap()]);
    }
}
