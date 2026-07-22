use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures_channel::oneshot;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen_futures::JsFuture;
use web_sys::DomException;
use web_sys::MidiAccess;
use web_sys::MidiConnectionEvent;
use web_sys::MidiInput;
use web_sys::MidiMessageEvent;
use web_sys::MidiOptions;
use web_sys::MidiOutput;
use web_sys::MidiPort;
use web_sys::MidiPortDeviceState;
use web_sys::MidiPortType;

use super::common::prune_send;
use super::common::Command;
use super::common::DestinationSubscribers;
use super::common::SourceSubscribers;
use super::common::StreamReceivers;
use super::common::StreamSenders;
use super::log_error;
use super::MutexExt;
use crate::midi::stream_parser::StreamParser;
use crate::name::Name;
use crate::time::Instant;
use crate::Destination;
use crate::DestinationChange;
use crate::Error;
use crate::IoError;
use crate::PortId;
use crate::Source;
use crate::SourceChange;

fn port_handle(js_id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in js_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn find_input(inputs: &js_sys::Map, handle: u64) -> Option<MidiInput> {
    let mut found = None;
    inputs.for_each(&mut |value, _key| {
        let input: MidiInput = value.unchecked_into();
        if port_handle(&input.id()) == handle {
            found = Some(input);
        }
    });
    found
}

fn find_output(outputs: &js_sys::Map, handle: u64) -> Option<MidiOutput> {
    let mut found = None;
    outputs.for_each(&mut |value, _key| {
        let output: MidiOutput = value.unchecked_into();
        if port_handle(&output.id()) == handle {
            found = Some(output);
        }
    });
    found
}

struct SourceConnection {
    input: MidiInput,
    _on_message: Closure<dyn FnMut(MidiMessageEvent)>,
}

struct WebState {
    cmd_rx: std::sync::mpsc::Receiver<Command>,
    source_subs: SourceSubscribers,
    destination_subs: DestinationSubscribers,
    access: Option<MidiAccess>,
    connections: HashMap<u64, SourceConnection>,
    destinations: HashMap<u64, MidiOutput>,
    _on_statechange: Option<Closure<dyn FnMut(MidiConnectionEvent)>>,
}

impl WebState {
    fn list_sources(&self) -> Vec<Source> {
        self.ports(MidiPortType::Input)
            .into_iter()
            .map(|(id, name)| Source {
                id,
                name,
                is_virtual: false,
            })
            .collect()
    }

    fn list_destinations(&self) -> Vec<Destination> {
        self.ports(MidiPortType::Output)
            .into_iter()
            .map(|(id, name)| Destination {
                id,
                name,
                is_virtual: false,
            })
            .collect()
    }

    fn ports(&self, kind: MidiPortType) -> Vec<(PortId, String)> {
        let Some(access) = &self.access else {
            return Vec::new();
        };
        let map: js_sys::Map = match kind {
            MidiPortType::Output => access.outputs().unchecked_into(),
            _ => access.inputs().unchecked_into(),
        };
        let mut out = Vec::new();
        map.for_each(&mut |value, _key| {
            let port: MidiPort = value.unchecked_into();
            out.push((
                PortId(port_handle(&port.id())),
                port.name().unwrap_or_default(),
            ));
        });
        out
    }

    fn connect_source(&mut self, port_id: PortId) -> Result<StreamReceivers, Error> {
        let handle = port_id.0;
        if self.connections.contains_key(&handle) {
            return Err(IoError::AlreadyConnected.into());
        }
        let Some(access) = self.access.clone() else {
            return Err(IoError::NotReady.into());
        };
        let inputs: js_sys::Map = access.inputs().unchecked_into();
        let Some(input) = find_input(&inputs, handle) else {
            return Err(IoError::PortNotFound.into());
        };
        let (senders, receivers) = StreamSenders::channel();
        let mut parser = StreamParser::new();
        let on_message =
            Closure::<dyn FnMut(MidiMessageEvent)>::new(move |ev: MidiMessageEvent| {
                match ev.data() {
                    Ok(bytes) => {
                        let timestamp = Instant::now();
                        parser.push(&bytes, &mut |event| senders.emit(timestamp, event));
                    }
                    Err(e) => log_error!("failed to read MIDI message data: {e:?}"),
                }
            });
        input.set_onmidimessage(Some(on_message.as_ref().unchecked_ref()));
        self.connections.insert(
            handle,
            SourceConnection {
                input,
                _on_message: on_message,
            },
        );
        Ok(receivers)
    }

    fn disconnect(&mut self, port_id: PortId) {
        if let Some(conn) = self.connections.remove(&port_id.0) {
            conn.input.set_onmidimessage(None);
        }
    }

    fn send(&self, port_id: PortId, data: &[u8]) -> Result<(), Error> {
        let Some(output) = self.destinations.get(&port_id.0) else {
            return Err(IoError::PortDisconnected.into());
        };
        let array = js_sys::Uint8Array::from(data);
        output
            .send(array.as_ref())
            .map_err(|e| map_web_error(e).into())
    }

    fn disconnect_destination(&mut self, port_id: PortId) {
        if let Some(output) = self.destinations.remove(&port_id.0) {
            let _ = output.close();
        }
    }
}

pub(super) struct Backend {
    state: Rc<RefCell<WebState>>,
}

impl Backend {
    pub(super) fn start(
        _name: Name,
        source_subs: SourceSubscribers,
        destination_subs: DestinationSubscribers,
        cmd_rx: std::sync::mpsc::Receiver<Command>,
        _cmd_tx: &std::sync::mpsc::SyncSender<Command>,
        ready_tx: oneshot::Sender<Result<(), Error>>,
    ) -> Result<Self, Error> {
        let state = Rc::new(RefCell::new(WebState {
            cmd_rx,
            source_subs,
            destination_subs,
            access: None,
            connections: HashMap::new(),
            destinations: HashMap::new(),
            _on_statechange: None,
        }));

        let init_state = Rc::clone(&state);
        spawn_local(async move {
            match request_access().await {
                Ok(access) => {
                    install_statechange(&init_state, &access);
                    init_state.borrow_mut().access = Some(access);
                    let _ = ready_tx.send(Ok(()));
                    drain(&init_state);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });

        Ok(Backend { state })
    }

    pub(super) fn wake(&self) {
        let state = Rc::clone(&self.state);
        spawn_local(async move {
            drain(&state);
        });
    }

    pub(super) fn on_drop(&self, _cmd_tx: &std::sync::mpsc::SyncSender<Command>) {
        let mut st = self.state.borrow_mut();
        if let Some(access) = &st.access {
            access.set_onstatechange(None);
        }
        for (_, conn) in st.connections.drain() {
            conn.input.set_onmidimessage(None);
        }
        for (_, output) in st.destinations.drain() {
            let _ = output.close();
        }
        st._on_statechange = None;
        st.access = None;
    }
}

async fn request_access() -> Result<MidiAccess, Error> {
    let window = web_sys::window().ok_or(IoError::Unsupported)?;
    let options = MidiOptions::new();
    options.set_sysex(true);
    let promise = window
        .navigator()
        .request_midi_access_with_options(&options)
        .map_err(map_web_error)?;
    let value = JsFuture::from(promise).await.map_err(map_web_error)?;
    Ok(value.unchecked_into::<MidiAccess>())
}

fn map_web_error(value: JsValue) -> IoError {
    let Ok(exception) = value.dyn_into::<DomException>() else {
        return IoError::Unsupported;
    };
    match exception.name().as_str() {
        "SecurityError" | "NotAllowedError" => IoError::PermissionDenied,
        "NotSupportedError" => IoError::Unsupported,
        "InvalidStateError" => IoError::PortDisconnected,
        _ => IoError::Web(exception.message()),
    }
}

fn connect_destination(
    state: &Rc<RefCell<WebState>>,
    port_id: PortId,
    reply: oneshot::Sender<Result<(), Error>>,
) {
    let handle = port_id.0;
    let state = Rc::clone(state);
    spawn_local(async move {
        let output = {
            let st = state.borrow();
            if st.destinations.contains_key(&handle) {
                let _ = reply.send(Err(IoError::AlreadyConnected.into()));
                return;
            }
            let Some(access) = st.access.clone() else {
                let _ = reply.send(Err(IoError::NotReady.into()));
                return;
            };
            let outputs: js_sys::Map = access.outputs().unchecked_into();
            let Some(output) = find_output(&outputs, handle) else {
                let _ = reply.send(Err(IoError::PortNotFound.into()));
                return;
            };
            output
        };
        match JsFuture::from(output.open()).await {
            Ok(_) => {
                state.borrow_mut().destinations.insert(handle, output);
                let _ = reply.send(Ok(()));
            }
            Err(e) => {
                let _ = reply.send(Err(map_web_error(e).into()));
            }
        }
    });
}

fn install_statechange(state: &Rc<RefCell<WebState>>, access: &MidiAccess) {
    let cb_state = Rc::clone(state);
    let closure = Closure::<dyn FnMut(MidiConnectionEvent)>::new(move |ev: MidiConnectionEvent| {
        handle_statechange(&cb_state, ev);
    });
    access.set_onstatechange(Some(closure.as_ref().unchecked_ref()));
    state.borrow_mut()._on_statechange = Some(closure);
}

fn handle_statechange(state: &Rc<RefCell<WebState>>, ev: MidiConnectionEvent) {
    let Some(port) = ev.port() else {
        return;
    };
    let handle = port_handle(&port.id());
    let mut st = state.borrow_mut();
    let connected = port.state() == MidiPortDeviceState::Connected;
    let name = port.name().unwrap_or_default();
    match port.type_() {
        MidiPortType::Input => {
            let source = Source {
                id: PortId(handle),
                name,
                is_virtual: false,
            };
            let change = if connected {
                SourceChange::Added(source)
            } else {
                SourceChange::Removed(source)
            };
            let mut guard = st.source_subs.lock_unpoisoned();
            prune_send(&mut guard, &change);
            drop(guard);
            if !connected {
                st.disconnect(PortId(handle));
            }
        }
        MidiPortType::Output => {
            let destination = Destination {
                id: PortId(handle),
                name,
                is_virtual: false,
            };
            let change = if connected {
                DestinationChange::Added(destination)
            } else {
                DestinationChange::Removed(destination)
            };
            let mut guard = st.destination_subs.lock_unpoisoned();
            prune_send(&mut guard, &change);
            drop(guard);
            if !connected {
                st.disconnect_destination(PortId(handle));
            }
        }
        _ => {}
    }
}

fn drain(state: &Rc<RefCell<WebState>>) {
    loop {
        let cmd = state.borrow_mut().cmd_rx.try_recv();
        match cmd {
            Ok(cmd) => process(state, cmd),
            Err(_) => break,
        }
    }
}

fn process(state: &Rc<RefCell<WebState>>, cmd: Command) {
    match cmd {
        Command::ListSources { reply } => {
            let sources = state.borrow_mut().list_sources();
            let _ = reply.send(Ok(sources));
        }
        Command::ListDestinations { reply } => {
            let destinations = state.borrow_mut().list_destinations();
            let _ = reply.send(Ok(destinations));
        }
        Command::ConnectSource { port_id, reply } => {
            let result = state.borrow_mut().connect_source(port_id);
            let _ = reply.send(result);
        }
        Command::Disconnect(port_id) => {
            state.borrow_mut().disconnect(port_id);
        }
        Command::ConnectDestination { port_id, reply } => {
            connect_destination(state, port_id, reply);
        }
        Command::SendMidi {
            port_id,
            msg,
            reply,
        } => {
            let result = state.borrow().send(port_id, &msg);
            let _ = reply.send(result);
        }
        Command::SendSysex {
            port_id,
            data,
            reply,
        } => {
            let result = state.borrow().send(port_id, &data);
            let _ = reply.send(result);
        }
        Command::CreateVirtualSource { reply, .. } => {
            let _ = reply.send(Err(IoError::Unsupported.into()));
        }
        Command::CreateVirtualDestination { reply, .. } => {
            let _ = reply.send(Err(IoError::Unsupported.into()));
        }
        Command::SendVirtualMidi { reply, .. } => {
            let _ = reply.send(Err(IoError::Unsupported.into()));
        }
        Command::SendVirtualSysex { reply, .. } => {
            let _ = reply.send(Err(IoError::Unsupported.into()));
        }
        Command::DisconnectDestination(port_id) => {
            state.borrow_mut().disconnect_destination(port_id);
        }
        Command::DestroyVirtualSource(_) => {}
        Command::DestroyVirtualDestination(_) => {}
    }
}
