//! Transmission for KAT Mobile: what desktop KAT's app does around its encoders and its HackRF
//! (`src/app.rs`'s `transmit_*`, `src/app/tx.rs`), on KAT's own code, for the Android app (JNI).
//!
//! - [decode]: a burst's decodes, with the fields an encoder needs ([Decode], which the app keeps
//!   on the capture).
//! - [tx_actions]: what a capture can send, as desktop's signal menu offers it: the car commands
//!   (Lock, Unlock, Trunk, Panic) for a car remote; "Send next code" and the remote's own buttons
//!   ([gate_buttons::tx_buttons]) for a gate, garage, barrier or alarm remote. Replay (the recorded
//!   pulses) is there for every capture.
//! - [plan]: one transmission: the recorded pulses (Replay), or the encoder's upload with the
//!   copies KAT sends ([crate::protocols::transmission]); the modulation the capture goes out with
//!   ([choose_tx_preset]); and, for a rolling code, the frame sent, which the app keeps on the
//!   capture once the transmission went out, so that the next press sends the code after
//!   ([sent_frame], [advances]).
//! - [tx_iq]: the HackRF's 8-bit IQ at 2 MS/s, the waveform [TX_LO_OFFSET_HZ] above the LO
//!   ([generate_tx_iq]). The app streams it to the radio (`HackRfUsb.kt`) the way desktop KAT's
//!   `transmit_signal_hackrf` does.

use std::sync::{Mutex, MutexGuard, OnceLock};

use jni::objects::{JClass, JIntArray, JString};
use jni::sys::{jboolean, jbyteArray, jdouble, jlong, jstring};
use jni::JNIEnv;
use serde::{Deserialize, Serialize};

use crate::capture::RfModulation;
use crate::protocols::info::Category;
use crate::protocols::{gate_buttons, DecodedSignal, ProtocolRegistry};
use crate::radio::preset::{choose_tx_preset, TxModulation};
use crate::radio::tx::{check_tx_air_time, generate_tx_iq, TX_LO_OFFSET_HZ, TX_SAMPLE_RATE};
use crate::radio::LevelDuration;

/// Longest transmission KAT Mobile sends: 10 s on air, 40 MB of IQ held in the phone's memory
/// (desktop KAT allows 60 s). KAT's longest protocol transmissions last a few seconds.
pub const MAX_MOBILE_TX_AIR_US: u64 = 10_000_000;

/// Desktop's car commands (`ButtonCommand`): the generic codes every car encoder takes.
const CAR_COMMANDS: [(u8, &str); 4] = [(0x01, "Lock"), (0x02, "Unlock"), (0x04, "Trunk"), (0x08, "Panic")];

/// A decode as the app keeps it on a capture: [DecodedSignal]'s fields, the 64-bit ones in hex (a
/// JSON number holds 53 bits).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Decode {
    /// The capture's protocol as KAT names it: the decode's display name ("KeeLoq (DoorHan)").
    pub protocol: String,
    pub serial: Option<u32>,
    pub button: Option<u8>,
    pub counter: Option<u16>,
    pub crc_valid: bool,
    /// The frame (`data`), hex.
    pub data: String,
    pub bits: usize,
    /// The protocol's encoder can build this remote's frames (desktop: `CaptureStatus::EncoderCapable`).
    pub encoder_capable: bool,
    /// Encoder state the frame does not carry (`extra`), hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
}

fn hex(value: &str) -> Result<u64, String> {
    u64::from_str_radix(value.trim().trim_start_matches("0x"), 16).map_err(|_| format!("not a hex number: {value}"))
}

impl Decode {
    pub fn new(protocol: &str, d: &DecodedSignal) -> Self {
        Self {
            protocol: protocol.to_string(),
            serial: d.serial,
            button: d.button,
            counter: d.counter,
            crc_valid: d.crc_valid,
            data: format!("{:X}", d.data),
            bits: d.data_count_bit,
            encoder_capable: d.encoder_capable,
            extra: d.extra.map(|e| format!("{e:X}")),
        }
    }

    /// The decode as the protocols take it (desktop's `Capture::decoded_signal`).
    pub fn signal(&self) -> Result<DecodedSignal, String> {
        Ok(DecodedSignal {
            serial: self.serial,
            button: self.button,
            counter: self.counter,
            crc_valid: self.crc_valid,
            data: hex(&self.data)?,
            data_count_bit: self.bits,
            encoder_capable: self.encoder_capable,
            extra: self.extra.as_deref().map(hex).transpose()?,
            protocol_display_name: Some(self.protocol.clone()),
        })
    }
}

/// One protocol that read a burst ([decode]).
#[derive(Serialize, Debug)]
pub struct Decoded {
    /// The decoder's name (the protocol listed).
    pub name: String,
    pub bits: usize,
    pub integrity: String,
    pub decode: Decode,
}

/// The protocols that read a burst (as [crate::identify]), each with its decode.
pub fn decode(durations: &[i32], frequency: u64, fm: bool, receive_path: bool) -> Result<Vec<Decoded>, String> {
    Ok(crate::scan(durations, frequency, fm, receive_path)?
        .into_iter()
        .map(|(r, name, d)| Decoded { decode: Decode::new(&name, &d), name: r.name, bits: r.bits, integrity: r.integrity })
        .collect())
}

/// The transmit side's own registry: [sent_frame] feeds a decoder, which must not disturb the
/// receiver's in the middle of a remote's frame.
fn tx_engine() -> MutexGuard<'static, ProtocolRegistry> {
    static TX_ENGINE: OnceLock<Mutex<ProtocolRegistry>> = OnceLock::new();
    TX_ENGINE.get_or_init(|| Mutex::new(crate::new_registry())).lock().unwrap_or_else(|e| e.into_inner())
}

/// What a capture can send besides Replay ([tx_actions]).
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct TxActions {
    /// The protocol's encoder can build this capture's frames.
    pub encodes: bool,
    /// A gate, garage, barrier or alarm remote: "Send next code" and its own buttons. A car remote
    /// gets the car commands instead.
    pub gate: bool,
    /// The buttons offered: the car commands, or the gate remote's own buttons (none when its
    /// protocol has no list; "Send next code" sends the captured one).
    pub buttons: Vec<TxButton>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct TxButton {
    pub code: u8,
    pub label: String,
}

/// Desktop's signal menu for a capture (`signal_actions_base`): the car commands for a car remote
/// whose encoder can build its frames, "Send next code" and the remote's buttons for a gate,
/// garage, barrier or alarm remote.
pub fn tx_actions(decode: &Decode) -> TxActions {
    let encodes = decode.encoder_capable && tx_engine().get(&decode.protocol).is_some_and(|p| p.supports_encoding());
    let gate = crate::protocols::is_keeloq_non_car(&decode.protocol)
        || crate::protocols::protocol_info(&decode.protocol).is_some_and(|i| i.category == Category::Gate);
    let buttons = if !encodes {
        Vec::new()
    } else if gate {
        match decode.signal() {
            Ok(d) => gate_buttons::tx_buttons(&decode.protocol, &d)
                .into_iter()
                .map(|code| TxButton { code, label: gate_buttons::button_label(&decode.protocol, code).to_string() })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        CAR_COMMANDS.iter().map(|&(code, label)| TxButton { code, label: label.to_string() }).collect()
    };
    TxActions { encodes, gate, buttons }
}

/// One transmission the app asks for ([plan]).
#[derive(Deserialize, Debug, Default)]
pub struct TxRequest {
    /// "replay": the recorded pulses; "next": the captured button's next code; "button": the next
    /// code of `button` (a car command or one of the remote's own buttons).
    pub action: String,
    #[serde(default)]
    pub button: Option<u8>,
    /// The capture's decode (needed to encode; Replay takes the protocol's preset from it).
    #[serde(default)]
    pub decode: Option<Decode>,
    /// Replay: the recorded pulses, µs, positive while high.
    #[serde(default)]
    pub pulses: Vec<i32>,
    pub frequency: u32,
    /// The demodulator path that received the capture: "AM" or "FM".
    #[serde(default)]
    pub received: Option<String>,
}

/// One transmission, ready for [tx_iq] and the radio.
#[derive(Serialize, Debug)]
pub struct TxPlan {
    /// What goes on air, µs, positive while the carrier is on (OOK) or above it (2-FSK).
    pub pulses: Vec<i32>,
    pub frequency: u32,
    /// "OOK" or "2-FSK ±47.60 kHz".
    pub modulation: String,
    pub fsk: bool,
    pub deviation_hz: f64,
    /// The preset this transmission stands for ("AM650", "FM476", …).
    pub preset: String,
    pub air_us: u64,
    /// The button sent (for "next", the captured one) and its name. Absent fields are left out of
    /// the JSON, not null (the app's JSON reader takes no null for an object).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_label: Option<String>,
    /// For a rolling code, the capture's decode after the frame sent (counter, frame, button,
    /// encoder state): the app keeps it once the transmission went out, so the next press sends
    /// the code after. None for Replay and fixed codes, which keep their capture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<Decode>,
}

/// Plan one transmission (desktop's `replay_capture`, `transmit_one_command` and `transmit_button`).
pub fn plan(req: &TxRequest) -> Result<TxPlan, String> {
    let received = match req.received.as_deref() {
        Some(r) if r.eq_ignore_ascii_case("FM") => Some(RfModulation::FM),
        Some(r) if r.eq_ignore_ascii_case("AM") => Some(RfModulation::AM),
        _ => None,
    };
    let presets = req.decode.as_ref().and_then(|d| crate::protocols::protocol_info(&d.protocol)).map(|i| i.presets);
    let (preset, modulation) = choose_tx_preset(None, received, presets);
    let (signal, button, sent) = match req.action.as_str() {
        "replay" => (pulses_of(&req.pulses)?, None, None),
        "next" | "button" => {
            let decode = req.decode.as_ref().ok_or("This capture has no decode to encode")?;
            let button = match req.action.as_str() {
                "button" => Some(req.button.ok_or("No button given")?),
                _ => None,
            };
            let (signal, pressed, sent) = encode(decode, button)?;
            (signal, Some(pressed), sent)
        }
        other => return Err(format!("Unknown transmit action: {other}")),
    };
    let air_us = air_time(&signal)?;
    let (fsk, deviation_hz) = match modulation {
        TxModulation::Ook => (false, 0.0),
        TxModulation::Fsk { deviation_hz } => (true, deviation_hz),
    };
    let protocol = req.decode.as_ref().map(|d| d.protocol.as_str()).unwrap_or("");
    Ok(TxPlan {
        pulses: signal.iter().map(|p| if p.level { p.duration_us as i32 } else { -(p.duration_us as i32) }).collect(),
        frequency: req.frequency,
        modulation: modulation.to_string(),
        fsk,
        deviation_hz,
        preset: preset.label(),
        air_us,
        button_label: button.map(|b| match req.action.as_str() {
            "button" if CAR_COMMANDS.iter().any(|&(c, _)| c == b) && !is_gate(protocol) => {
                CAR_COMMANDS.iter().find(|&&(c, _)| c == b).map(|&(_, l)| l.to_string()).unwrap_or_default()
            }
            _ => gate_buttons::button_label(protocol, b).to_string(),
        }),
        button,
        sent,
    })
}

fn is_gate(protocol: &str) -> bool {
    crate::protocols::is_keeloq_non_car(protocol)
        || crate::protocols::protocol_info(protocol).is_some_and(|i| i.category == Category::Gate)
}

/// The encoder's transmission for `decode` with `button` (None: the captured button's next
/// code), the button pressed, and the capture's decode after it when the code rolls.
fn encode(decode: &Decode, button: Option<u8>) -> Result<(Vec<LevelDuration>, u8, Option<Decode>), String> {
    if !decode.encoder_capable {
        return Err(format!("{}: KAT cannot build this remote's frames (its key is not known)", decode.protocol));
    }
    // As desktop builds it from the capture: the decode, encoder-capable, with its display name.
    let from = DecodedSignal { encoder_capable: true, ..decode.signal()? };
    let mut registry = tx_engine();
    let upload = {
        let p = registry
            .get(&decode.protocol)
            .ok_or_else(|| format!("Protocol {} is not supported for encoding", decode.protocol))?;
        if !p.supports_encoding() {
            return Err(format!("Protocol {} does not support encoding", decode.protocol));
        }
        crate::protocols::transmission(p, &from, button, None)
            .ok_or_else(|| format!("{}: the encoder made no frame for this capture", decode.protocol))?
    };
    let pressed = button.or(from.button).unwrap_or(0);
    let sent = match sent_frame(&mut registry, &decode.protocol, &from, pressed, &upload) {
        Some(s) if advances(&registry, &decode.protocol, &from, &s) => Some(store_sent_frame(decode, &s)),
        _ => None,
    };
    Ok((upload, pressed, sent))
}

/// `upload` on air, sent twice with a gap: adjacent pulses of the same level are one pulse to a
/// receiver (`merge`). Desktop's `app/tx.rs` `on_air`.
fn on_air(upload: &[LevelDuration], merge: bool) -> Vec<LevelDuration> {
    let gap = [LevelDuration::new(false, 20_000)];
    let mut stream: Vec<LevelDuration> = Vec::with_capacity(2 * upload.len() + 2);
    for p in upload.iter().chain(&gap).chain(upload).chain(&gap) {
        match stream.last_mut() {
            Some(last) if merge && last.level == p.level => last.duration_us += p.duration_us,
            _ => stream.push(LevelDuration::new(p.level, p.duration_us)),
        }
    }
    stream
}

/// The frame `upload` carries: what the encoder reports building, else the last frame the
/// protocol's own decoder reads from it with the capture's serial, completed with the encoder
/// state the air does not carry. Desktop's `app/tx.rs` `sent_frame`.
fn sent_frame(
    registry: &mut ProtocolRegistry,
    protocol: &str,
    from: &DecodedSignal,
    button: u8,
    upload: &[LevelDuration],
) -> Option<DecodedSignal> {
    if let Some(sent) = registry.get(protocol).and_then(|p| p.sent_signal(from, button)) {
        return Some(sent);
    }
    let sent = [true, false].into_iter().find_map(|merge| {
        registry
            .decode_with(protocol, &on_air(upload, merge))
            .into_iter()
            .rev()
            .find(|d| from.serial.is_none() || d.serial == from.serial)
    })?;
    Some(with_context(registry, protocol, from, sent))
}

/// Encoder state a frame does not carry on air stays the capture's; when the frame alone does not
/// give the counter, the protocol reads it again with that state through its key file. Desktop's
/// `app/tx.rs` `with_context`.
fn with_context(registry: &ProtocolRegistry, protocol: &str, from: &DecodedSignal, sent: DecodedSignal) -> DecodedSignal {
    let sent = DecodedSignal { extra: sent.extra.or(from.extra), ..sent };
    if sent.counter.is_some() || from.counter.is_none() {
        return sent;
    }
    registry
        .get(protocol)
        .and_then(|p| p.to_flipper_key(&sent).and_then(|key| p.from_flipper_key(&key)))
        .filter(|d| d.serial == sent.serial)
        .unwrap_or(sent)
}

/// Whether the frame sent is the remote's next press (desktop's `app/tx.rs` `advances`).
fn advances(registry: &ProtocolRegistry, protocol: &str, from: &DecodedSignal, sent: &DecodedSignal) -> bool {
    registry
        .get(protocol)
        .map_or_else(|| crate::protocols::counter_moved(from, sent), |p| p.advances(from, sent))
}

/// The capture's decode once the frame sent is kept on it: counter, frame, button and encoder
/// state; the protocol name stays. Desktop's `app/tx.rs` `store_sent_frame`.
fn store_sent_frame(capture: &Decode, sent: &DecodedSignal) -> Decode {
    Decode {
        protocol: capture.protocol.clone(),
        serial: sent.serial.or(capture.serial),
        button: sent.button.or(capture.button),
        counter: sent.counter,
        crc_valid: sent.crc_valid,
        data: format!("{:X}", sent.data),
        bits: sent.data_count_bit,
        encoder_capable: capture.encoder_capable,
        extra: sent.extra.map(|e| format!("{e:X}")),
    }
}

/// Pulses as the app holds them (µs, positive while high) as level/duration pairs.
fn pulses_of(durations: &[i32]) -> Result<Vec<LevelDuration>, String> {
    if durations.is_empty() {
        return Err("No pulses to send".into());
    }
    if durations.len() > 1 << 20 {
        return Err("Too many pulses".into());
    }
    durations
        .iter()
        .map(|&v| {
            let d = v.unsigned_abs();
            if d == 0 || u64::from(d) > MAX_MOBILE_TX_AIR_US {
                Err(format!("Invalid pulse duration: {v} µs"))
            } else {
                Ok(LevelDuration::new(v > 0, d))
            }
        })
        .collect()
}

/// The signal's air time, or why KAT Mobile does not send it.
fn air_time(signal: &[LevelDuration]) -> Result<u64, String> {
    let us = check_tx_air_time(signal).map_err(|e| e.to_string())?;
    if us > MAX_MOBILE_TX_AIR_US {
        return Err(format!(
            "Transmission refused: {:.1} s on air, more than the {} s KAT Mobile sends",
            us as f64 / 1e6,
            MAX_MOBILE_TX_AIR_US / 1_000_000
        ));
    }
    Ok(us)
}

/// The HackRF's samples for `pulses`: 8-bit I, Q pairs at [TX_SAMPLE_RATE], the waveform
/// [TX_LO_OFFSET_HZ] above the LO (OOK, or continuous-phase 2-FSK at ±`deviation_hz`).
pub fn tx_iq(pulses: &[i32], fsk: bool, deviation_hz: f64) -> Result<Vec<u8>, String> {
    let signal = pulses_of(pulses)?;
    air_time(&signal)?;
    let modulation = if fsk {
        if !(deviation_hz.is_finite() && deviation_hz > 0.0 && deviation_hz < 200_000.0) {
            return Err(format!("Invalid 2-FSK deviation: {deviation_hz} Hz"));
        }
        TxModulation::Fsk { deviation_hz }
    } else {
        TxModulation::Ook
    };
    let iq = generate_tx_iq(&signal, modulation, TX_SAMPLE_RATE, TX_LO_OFFSET_HZ);
    let mut bytes = Vec::with_capacity(iq.len() * 2);
    for s in iq {
        bytes.push(s.re as u8);
        bytes.push(s.im as u8);
    }
    Ok(bytes)
}

/// The decode a KAT `.fob` file holds (desktop KAT exports its capture's decode in it, after
/// each transmission's counter too), or None when it has no protocol frame.
pub fn fob_decode(text: &str) -> Option<Decode> {
    let capture = crate::interop_fob::parse_fob(text, 0).ok()?;
    let protocol = capture.protocol.clone()?;
    (capture.data_count_bit > 0).then(|| Decode::new(&protocol, &capture.decoded_signal()))
}

/// Test support: whether KAT's embedded keys are loaded (the Kia and VAG keys among them).
#[doc(hidden)]
pub fn embedded_keys_loaded() -> bool {
    drop(tx_engine());
    let keys = crate::protocols::keys::get_keystore();
    keys.kia_mf_key != 0 && keys.vag_keys_loaded && !keys.vag_keys.is_empty()
}

/// Test support: KAT's "Add manually" remotes (`protocols::add_manually`), as decodes, for the
/// types KAT can create with its embedded keys: (type id, decode, frequency).
#[doc(hidden)]
pub fn manual_remotes(seed: u32) -> Vec<(String, Decode, u32)> {
    use crate::protocols::add_manually::{availability, fill, generate, Scripted, TYPES};
    let registry = tx_engine();
    let mut out = Vec::new();
    for ty in TYPES {
        if availability(ty).is_err() {
            continue;
        }
        let mut rng = Scripted::new(&[seed, seed.wrapping_mul(2654435761), seed ^ 0x5EED_0001]);
        let gen = fill(ty, &mut rng);
        let Ok(key) = generate(&gen, &mut rng) else { continue };
        let protocol = key.protocol.as_str();
        let Some(d) = registry.get(protocol).and_then(|p| p.from_flipper_key(&key)) else { continue };
        let name = d.protocol_display_name.clone().unwrap_or_else(|| protocol.to_string());
        out.push((ty.id.to_string(), Decode::new(&name, &d), key.frequency));
    }
    out
}

// ── JNI ──────────────────────────────────────────────────────────────────────────────────────────

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

fn jstring_of(env: &mut JNIEnv, text: String) -> jstring {
    env.new_string(text).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn string_arg(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    let s: String = env.get_string(value).map_err(|e| e.to_string())?.into();
    if s.len() > 8 * 1024 * 1024 {
        return Err("Input too large".into());
    }
    Ok(s)
}

fn int_array(env: &mut JNIEnv, input: &JIntArray, max: usize) -> Result<Vec<i32>, String> {
    let len = env.get_array_length(input).map_err(|e| e.to_string())? as usize;
    if len > max {
        return Err("Input outside bounds".into());
    }
    let mut values = vec![0i32; len];
    env.get_int_array_region(input, 0, &mut values).map_err(|e| e.to_string())?;
    Ok(values)
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeDecoder_nativeDecode(
    mut env: JNIEnv,
    _class: JClass,
    input: JIntArray,
    frequency: jlong,
    fm: jboolean,
    receive_path: jboolean,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<Decoded>, String> {
        if frequency < 0 {
            return Err("Invalid frequency".into());
        }
        let values = int_array(&mut env, &input, 32768)?;
        decode(&values, frequency as u64, fm != 0, receive_path != 0)
    }))
    .unwrap_or_else(|_| Err("Decoder input was rejected".into()));
    let text = result.map(|v| json(&v)).unwrap_or_else(|_| "[]".into());
    jstring_of(&mut env, text)
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeTx_nativeActions(mut env: JNIEnv, _class: JClass, decode: JString) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<TxActions, String> {
        let text = string_arg(&mut env, &decode)?;
        let decode: Decode = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(tx_actions(&decode))
    }))
    .unwrap_or_else(|_| Err("rejected".into()));
    let text = json(&result.unwrap_or(TxActions { encodes: false, gate: false, buttons: Vec::new() }));
    jstring_of(&mut env, text)
}

#[derive(Serialize)]
struct PlanResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<TxPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeTx_nativePlan(mut env: JNIEnv, _class: JClass, request: JString) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<TxPlan, String> {
        let text = string_arg(&mut env, &request)?;
        let request: TxRequest = serde_json::from_str(&text).map_err(|e| format!("Invalid request: {e}"))?;
        plan(&request)
    }))
    .unwrap_or_else(|_| Err("The encoder failed on this capture".into()));
    let out = match result {
        Ok(plan) => PlanResult { ok: true, plan: Some(plan), error: None },
        Err(e) => PlanResult { ok: false, plan: None, error: Some(e) },
    };
    jstring_of(&mut env, json(&out))
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeTx_nativeIq(
    mut env: JNIEnv,
    _class: JClass,
    pulses: JIntArray,
    fsk: jboolean,
    deviation_hz: jdouble,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<u8>, String> {
        let values = int_array(&mut env, &pulses, 1 << 20)?;
        tx_iq(&values, fsk != 0, deviation_hz)
    }))
    .unwrap_or_else(|_| Err("IQ generation failed".into()));
    match result.and_then(|bytes| env.byte_array_from_slice(&bytes).map_err(|e| e.to_string())) {
        Ok(array) => array.into_raw(),
        Err(e) => {
            let _ = env.throw_new("java/lang/IllegalArgumentException", e);
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeTx_nativeFobDecode(mut env: JNIEnv, _class: JClass, text: JString) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<Decode> {
        let text = string_arg(&mut env, &text).ok()?;
        fob_decode(&text)
    }))
    .unwrap_or(None);
    let out = result.map(|d| json(&d)).unwrap_or_default();
    jstring_of(&mut env, out)
}
