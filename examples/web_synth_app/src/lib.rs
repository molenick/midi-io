use std::collections::HashMap;

use midi_io::Client;
use midi_io::Decoded;
use midi_io::MidiMessage;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::AudioContext;
use web_sys::GainNode;
use web_sys::OscillatorNode;
use web_sys::OscillatorType;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    if is_firefox() {
        reveal("firefox-note");
    }
    status("Click Start, then play your MIDI controller.");
}

#[wasm_bindgen]
pub async fn run() {
    set_started(true);
    if let Err(e) = play().await {
        status(&e);
        set_started(false);
    }
}

fn is_firefox() -> bool {
    web_sys::window()
        .and_then(|window| window.navigator().user_agent().ok())
        .is_some_and(|agent| agent.contains("Firefox"))
}

async fn play() -> Result<(), String> {
    status("Requesting MIDI access\u{2026}");
    let client = Client::new("web-synth").await.map_err(fmt)?;
    let sources = client.sources().await.map_err(fmt)?;
    let Some(source) = sources.into_iter().next() else {
        return Err("No MIDI sources found. Plug in a controller and reload.".to_string());
    };

    status(&format!("Playing: {}", source.name()));
    let connection = client.connect_source(&source).await.map_err(fmt)?;
    let context = AudioContext::new().map_err(js)?;
    let mut voices: HashMap<u8, (OscillatorNode, GainNode)> = HashMap::new();

    let mut events = connection.into_events();
    while let Some(timed) = events.recv().await {
        match timed.payload {
            Ok(Decoded::Message(MidiMessage::NoteOn { key, velocity, .. }))
                if velocity.get() > 0 =>
            {
                note_on(&context, &mut voices, key.get(), velocity.get())?;
            }
            Ok(Decoded::Message(MidiMessage::NoteOff { key, .. })) => {
                note_off(&context, &mut voices, key.get());
            }
            Ok(Decoded::Message(MidiMessage::ControlChange { controller, .. }))
                if controller.get() == 120 || controller.get() == 123 =>
            {
                release_all(&context, &mut voices);
            }
            _ => {}
        }
    }
    Ok(())
}

const ATTACK: f64 = 0.005;
const RELEASE: f64 = 0.03;

fn note_on(
    context: &AudioContext,
    voices: &mut HashMap<u8, (OscillatorNode, GainNode)>,
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

fn note_off(context: &AudioContext, voices: &mut HashMap<u8, (OscillatorNode, GainNode)>, key: u8) {
    if let Some(voice) = voices.remove(&key) {
        release(context, voice);
    }
}

fn release_all(context: &AudioContext, voices: &mut HashMap<u8, (OscillatorNode, GainNode)>) {
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

fn fmt<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn js(value: wasm_bindgen::JsValue) -> String {
    format!("{value:?}")
}

fn status(message: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("status"))
    {
        element.set_text_content(Some(message));
    }
}

fn set_started(started: bool) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("start"))
    {
        if started {
            let _ = element.set_attribute("disabled", "");
        } else {
            let _ = element.remove_attribute("disabled");
        }
    }
}

fn reveal(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        let _ = element.remove_attribute("hidden");
    }
}
