use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

use midi_io::time::Instant;
use midi_io::Client;
use midi_io::Decoded;
use midi_io::DestinationChange;
use midi_io::MidiMessage;
use midi_io::PortId;
use midi_io::Source;
use midi_io::SourceChange;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioContext;
use web_sys::AudioContextState;
use web_sys::Document;
use web_sys::Element;
use web_sys::GainNode;
use web_sys::HtmlElement;
use web_sys::OscillatorNode;
use web_sys::OscillatorType;

type Voices = HashMap<u8, (OscillatorNode, GainNode)>;

struct App {
    client: Option<Client>,
    context: Option<AudioContext>,
    start: Instant,
    connected: HashSet<PortId>,
    sources: Vec<(PortId, String)>,
    destinations: Vec<(PortId, String)>,
    receiving: bool,
}

#[derive(Clone, Copy)]
enum Support {
    Yes,
    No,
    Pending,
}

impl Support {
    fn class(self) -> &'static str {
        match self {
            Support::Yes => "yes",
            Support::No => "no",
            Support::Pending => "pending",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Support::Yes => "Yes",
            Support::No => "No",
            Support::Pending => "Pending",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Support::Yes => "\u{2713}",
            Support::No => "\u{2717}",
            Support::Pending => "\u{2026}",
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    render_support();
    if !has_web_midi() {
        reveal("unsupported");
        set_sound_button(false);
        return;
    }
    let app = Rc::new(RefCell::new(App {
        client: None,
        context: None,
        start: Instant::now(),
        connected: HashSet::new(),
        sources: Vec::new(),
        destinations: Vec::new(),
        receiving: false,
    }));
    APP.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&app)));
    spawn_local(async move {
        if let Err(e) = play(&app).await {
            status(&e);
            set_sound_button(false);
        }
    });
}

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
pub async fn enable_sound() {
    let Some(app) = APP.with(|cell| cell.borrow().clone()) else {
        return;
    };
    set_sound_button(false);
    let existing = app.borrow().context.clone();
    let context = match existing {
        Some(context) => context,
        None => match AudioContext::new() {
            Ok(context) => context,
            Err(e) => {
                sound_failed(&js_message(&e));
                return;
            }
        },
    };
    let resumed = match context.resume() {
        Ok(promise) => JsFuture::from(promise).await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = resumed {
        if app.borrow().context.is_none() {
            let _ = context.close();
        }
        sound_failed(&js_message(&e));
        return;
    }
    if context.state() != AudioContextState::Running {
        if app.borrow().context.is_none() {
            let _ = context.close();
        }
        sound_failed(&format!("{:?}", context.state()).to_lowercase());
        return;
    }
    if app.borrow().context.is_none() {
        watch_sound(&app, &context);
        app.borrow_mut().context = Some(context);
    }
    sound_running();
}

fn watch_sound(app: &Rc<RefCell<App>>, context: &AudioContext) {
    let app = Rc::clone(app);
    let closure = Closure::<dyn FnMut()>::new(move || {
        let Some(context) = app.borrow().context.clone() else {
            return;
        };
        if context.state() == AudioContextState::Running {
            sound_running();
        } else {
            sound_stopped();
        }
    });
    context.set_onstatechange(Some(closure.as_ref().unchecked_ref()));
    closure.forget();
}

fn sound_running() {
    set_support("support-sound", Support::Yes, "Running");
    clear_sound_status();
    if let Some(element) = element("enable-sound") {
        let _ = element.class_list().add_1("reserved");
    }
    if let Some(Ok(log)) = element("log").map(|log| log.dyn_into::<HtmlElement>()) {
        let _ = log.focus();
    }
}

fn sound_stopped() {
    set_support("support-sound", Support::No, "Stopped");
    status("Sound stopped");
    show_sound_button();
}

fn sound_failed(error: &str) {
    set_support("support-sound", Support::No, error);
    status(&format!("Sound didn't start: {error}"));
    show_sound_button();
}

fn show_sound_button() {
    if let Some(element) = element("enable-sound") {
        let _ = element.class_list().remove_1("reserved");
    }
    set_sound_button(true);
}

fn clear_sound_status() {
    let is_sound = element("status")
        .and_then(|element| element.text_content())
        .is_some_and(|text| text.starts_with("Sound"));
    if is_sound {
        status("");
    }
}

fn js_message(value: &wasm_bindgen::JsValue) -> String {
    if let Some(text) = value.as_string() {
        return text;
    }
    js_sys::Reflect::get(value, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| js(value.clone()))
}

fn has_web_midi() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::has(&window.navigator(), &"requestMIDIAccess".into()).unwrap_or(false)
    })
}

fn render_support() {
    if has_web_midi() {
        set_support("support-api", Support::Yes, "Supported");
        set_support("support-permission", Support::Pending, "");
        set_support("support-sources", Support::Pending, "");
        set_support("support-destinations", Support::Pending, "");
        set_support("support-receiving", Support::Pending, "");
        set_support("support-sound", Support::Pending, "Click Enable sound");
    } else {
        set_support("support-api", Support::No, "Not supported");
        for id in [
            "support-permission",
            "support-sources",
            "support-destinations",
            "support-receiving",
            "support-sound",
        ] {
            set_support(id, Support::No, "");
        }
    }
}

fn set_support(id: &str, support: Support, note: &str) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(cell)) = document.query_selector(&format!("#{id} td:first-child")) {
        fill_support(&document, &cell, support);
    }
    if let Ok(Some(cell)) = document.query_selector(&format!("#{id} td:last-child")) {
        cell.set_text_content(Some(note));
    }
}

fn fill_support(document: &Document, cell: &Element, support: Support) {
    cell.set_text_content(None);
    let _ = cell.set_attribute("class", support.class());
    if let Ok(icon) = document.create_element("span") {
        icon.set_text_content(Some(support.icon()));
        let _ = icon.set_attribute("aria-hidden", "true");
        let _ = cell.append_child(&icon);
    }
    if let Ok(text) = document.create_element("span") {
        text.set_text_content(Some(support.text()));
        let _ = text.set_attribute("class", "visually-hidden");
        let _ = cell.append_child(&text);
    }
}

async fn play(app: &Rc<RefCell<App>>) -> Result<(), String> {
    status("Requesting MIDI access\u{2026}");
    set_support("support-permission", Support::Pending, "Requesting");
    let client = match Client::new("web-synth").await {
        Ok(client) => client,
        Err(e) => {
            set_support("support-permission", Support::No, &e.to_string());
            for id in [
                "support-sources",
                "support-destinations",
                "support-receiving",
                "support-sound",
            ] {
                set_support(id, Support::No, "");
            }
            return Err(e.to_string());
        }
    };
    set_support("support-permission", Support::Yes, "Granted");
    status("");
    let mut source_changes = client.source_changes();
    let mut destination_changes = client.destination_changes();
    let sources = client.sources().await.map_err(fmt)?;
    let destinations = client.destinations().await.map_err(fmt)?;
    app.borrow_mut().client = Some(client);
    render_ports(&app.borrow());

    for source in sources {
        add_source(app, source);
    }
    for destination in destinations {
        add_destination(app, destination.id(), destination.name());
    }

    let destination_app = Rc::clone(app);
    spawn_local(async move {
        while let Some(change) = destination_changes.recv().await {
            match change {
                DestinationChange::Added(destination) => {
                    add_destination(&destination_app, destination.id(), destination.name());
                }
                DestinationChange::Removed(destination) => {
                    remove_destination(&destination_app, destination.id(), destination.name());
                }
            }
        }
    });

    while let Some(change) = source_changes.recv().await {
        match change {
            SourceChange::Added(source) => add_source(app, source),
            SourceChange::Removed(source) => remove_source(app, source.id(), source.name()),
        }
    }
    Ok(())
}

fn add_source(app: &Rc<RefCell<App>>, source: Source) {
    {
        let mut state = app.borrow_mut();
        if !state.sources.iter().any(|(id, _)| *id == source.id()) {
            state.sources.push((source.id(), source.name().to_string()));
            let line = format!("{} + source {}", stamp(state.start), source.name());
            log(&line);
            render_ports(&state);
        }
        if !state.connected.insert(source.id()) {
            return;
        }
    }
    let app = Rc::clone(app);
    spawn_local(async move {
        let Some(client) = app.borrow().client.clone() else {
            return;
        };
        let connection = match client.connect_source(&source).await {
            Ok(connection) => connection,
            Err(e) => {
                let mut state = app.borrow_mut();
                state.connected.remove(&source.id());
                log(&format!("{} {}: {e}", stamp(state.start), source.name()));
                return;
            }
        };
        let mut voices = Voices::new();
        let mut events = connection.into_events();
        while let Some(timed) = events.recv().await {
            if !app.borrow().receiving {
                app.borrow_mut().receiving = true;
                set_support("support-receiving", Support::Yes, source.name());
            }
            let state = app.borrow();
            let line = match &timed.payload {
                Ok(Decoded::Message(message)) => describe(message),
                Ok(Decoded::SysEx(sysex)) => hex(sysex),
                Err(e) => format!("error: {e}"),
            };
            log(&format!(
                "{} {line}  {}",
                stamp_at(state.start, timed.timestamp),
                source.name()
            ));
            if let Ok(Decoded::Message(message)) = timed.payload {
                if let Some(context) = &state.context {
                    if let Err(e) = synth(context, &mut voices, message) {
                        log(&format!("{} {e}", stamp(state.start)));
                    }
                }
            }
        }
        if let Some(context) = &app.borrow().context {
            release_all(context, &mut voices);
        }
    });
}

fn remove_source(app: &Rc<RefCell<App>>, id: PortId, name: &str) {
    let mut state = app.borrow_mut();
    state.connected.remove(&id);
    let before = state.sources.len();
    state.sources.retain(|(port, _)| *port != id);
    if state.sources.len() != before {
        log(&format!("{} - source {name}", stamp(state.start)));
        render_ports(&state);
    }
}

fn add_destination(app: &Rc<RefCell<App>>, id: PortId, name: &str) {
    let mut state = app.borrow_mut();
    if state.destinations.iter().any(|(port, _)| *port == id) {
        return;
    }
    state.destinations.push((id, name.to_string()));
    log(&format!("{} + destination {name}", stamp(state.start)));
    render_ports(&state);
}

fn remove_destination(app: &Rc<RefCell<App>>, id: PortId, name: &str) {
    let mut state = app.borrow_mut();
    let before = state.destinations.len();
    state.destinations.retain(|(port, _)| *port != id);
    if state.destinations.len() != before {
        log(&format!("{} - destination {name}", stamp(state.start)));
        render_ports(&state);
    }
}

fn synth(context: &AudioContext, voices: &mut Voices, message: MidiMessage) -> Result<(), String> {
    match message {
        MidiMessage::NoteOn { key, velocity, .. } if velocity.get() > 0 => {
            note_on(context, voices, key.get(), velocity.get())?;
        }
        MidiMessage::NoteOff { key, .. } => note_off(context, voices, key.get()),
        MidiMessage::ControlChange { controller, .. }
            if controller.get() == 120 || controller.get() == 123 =>
        {
            release_all(context, voices);
        }
        _ => {}
    }
    Ok(())
}

const ATTACK: f64 = 0.005;
const RELEASE: f64 = 0.03;

fn note_on(
    context: &AudioContext,
    voices: &mut Voices,
    key: u8,
    velocity: u8,
) -> Result<(), String> {
    note_off(context, voices, key);
    let now = context.current_time();
    let oscillator = context.create_oscillator().map_err(js)?;
    let gain = context.create_gain().map_err(js)?;
    oscillator.set_type(OscillatorType::Sine);
    oscillator.frequency().set_value(frequency(key));
    let level = gain.gain();
    level.set_value_at_time(0.0, now).map_err(js)?;
    level
        .linear_ramp_to_value_at_time(f32::from(velocity) / 127.0 * 0.2, now + ATTACK)
        .map_err(js)?;
    oscillator.connect_with_audio_node(&gain).map_err(js)?;
    gain.connect_with_audio_node(&context.destination())
        .map_err(js)?;
    oscillator.start().map_err(js)?;
    voices.insert(key, (oscillator, gain));
    Ok(())
}

fn note_off(context: &AudioContext, voices: &mut Voices, key: u8) {
    if let Some(voice) = voices.remove(&key) {
        release(context, voice);
    }
}

fn release_all(context: &AudioContext, voices: &mut Voices) {
    for (_, voice) in voices.drain() {
        release(context, voice);
    }
}

fn release(context: &AudioContext, (oscillator, gain): (OscillatorNode, GainNode)) {
    let now = context.current_time();
    let level = gain.gain();
    let _ = level.cancel_scheduled_values(now);
    let _ = level.set_value_at_time(level.value(), now);
    let _ = level.linear_ramp_to_value_at_time(0.0, now + RELEASE);
    let _ = oscillator.stop_with_when(now + RELEASE);
}

fn frequency(key: u8) -> f32 {
    440.0 * 2.0f32.powf((f32::from(key) - 69.0) / 12.0)
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn note_name(key: u8) -> String {
    let octave = i32::from(key / 12) - 1;
    format!("{}{octave}", NOTE_NAMES[usize::from(key % 12)])
}

fn describe(message: &MidiMessage) -> String {
    match message {
        MidiMessage::NoteOn {
            channel,
            key,
            velocity,
        } => format!(
            "NoteOn  {:<4} ({:>3})  vel {:>3}  ch {channel}",
            note_name(key.get()),
            key.get(),
            velocity.get()
        ),
        MidiMessage::NoteOff {
            channel,
            key,
            velocity,
        } => format!(
            "NoteOff {:<4} ({:>3})  vel {:>3}  ch {channel}",
            note_name(key.get()),
            key.get(),
            velocity.get()
        ),
        other => other.to_string(),
    }
}

fn hex(sysex: &midi_io::SysEx) -> String {
    let bytes: Vec<String> = Decoded::SysEx(sysex.clone())
        .to_wire_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    format!("SysEx {}", bytes.join(" "))
}

fn stamp(start: Instant) -> String {
    stamp_at(start, Instant::now())
}

fn stamp_at(start: Instant, at: Instant) -> String {
    format!(
        "[{:>9.3}]",
        at.saturating_duration_since(start).as_secs_f64()
    )
}

fn fmt<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn js(value: wasm_bindgen::JsValue) -> String {
    format!("{value:?}")
}

fn element(id: &str) -> Option<Element> {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
}

const LOG_LINES: u32 = 500;

fn log(line: &str) {
    let Some(log) = element("log") else {
        return;
    };
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let follow =
        log.scroll_top() + f64::from(log.client_height()) >= f64::from(log.scroll_height()) - 4.0;
    if let Ok(entry) = document.create_element("div") {
        entry.set_text_content(Some(line));
        let _ = log.append_child(&entry);
    }
    while log.child_element_count() > LOG_LINES {
        if let Some(first) = log.first_element_child() {
            first.remove();
        }
    }
    if follow {
        log.set_scroll_top(f64::from(log.scroll_height()));
    }
}

fn render_ports(app: &App) {
    render_port_row("support-sources", &app.sources);
    render_port_row("support-destinations", &app.destinations);
    if !app.receiving {
        set_support("support-receiving", Support::Pending, "Play a note");
    }
}

fn render_port_row(id: &str, ports: &[(PortId, String)]) {
    if ports.is_empty() {
        set_support(id, Support::No, "None");
        return;
    }
    let names: Vec<&str> = ports.iter().map(|(_, name)| name.as_str()).collect();
    set_support(id, Support::Yes, &names.join("\n"));
}

fn status(message: &str) {
    if let Some(element) = element("status") {
        element.set_text_content(Some(message));
    }
}

fn set_sound_button(enabled: bool) {
    if let Some(element) = element("enable-sound") {
        if enabled {
            let _ = element.remove_attribute("disabled");
        } else {
            let _ = element.set_attribute("disabled", "");
        }
    }
}

fn reveal(id: &str) {
    if let Some(element) = element(id) {
        let _ = element.remove_attribute("hidden");
    }
}
