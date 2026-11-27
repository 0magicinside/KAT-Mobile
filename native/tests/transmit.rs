//! The transmit side of the core (`mobile_tx`): plans, KAT's encoders, IQ, and the IQ received back
//! through the receive pipeline. Synthetic signals and KAT's "Add manually" remotes only: no
//! recording of a real remote.

use kat_rx::mobile_tx::{decode, embedded_keys_loaded, manual_remotes, plan, tx_actions, tx_iq, Decode, TxRequest};
use kat_rx::ReceiveSession;

const F433: u32 = 433_920_000;

/// A Princeton remote's press: 4 frames of 24 bits behind a long gap.
fn princeton() -> Vec<i32> {
    let mut p = vec![-11700];
    for _ in 0..4 {
        for c in "101001011001011010010110".chars() {
            p.extend(if c == '1' { [1170, -390] } else { [390, -1170] });
        }
        p.extend([390, -11700]);
    }
    p
}

fn request(action: &str, decode: Option<&Decode>, button: Option<u8>) -> TxRequest {
    TxRequest {
        action: action.into(),
        button,
        decode: decode.cloned(),
        pulses: Vec::new(),
        frequency: F433,
        received: Some("AM".into()),
    }
}

/// The receiver's bursts for `iq` (8-bit IQ at 2 MS/s with the carrier 500 kHz above the LO, as the
/// HackRF sends it), behind and before some silence.
fn receive(iq: &[u8]) -> Vec<Burst> {
    let mut rx = ReceiveSession::with_offset(2_000_000, F433, 500_000.0);
    let silence = vec![0u8; 2 * 40_000];
    let mut bursts = Vec::new();
    for chunk in [&silence[..], iq, &silence[..], &silence[..], &silence[..]] {
        for block in chunk.chunks(65_536) {
            bursts.extend(rx.process(block).bursts.into_iter().map(|b| Burst { modulation: b.modulation, pulses: b.pulses }));
        }
    }
    bursts.extend(rx.flush().bursts.into_iter().map(|b| Burst { modulation: b.modulation, pulses: b.pulses }));
    bursts
}

struct Burst {
    modulation: String,
    pulses: Vec<i32>,
}

#[test]
fn the_embedded_keys_are_loaded_as_on_the_desktop() {
    assert!(embedded_keys_loaded(), "KAT's embedded keystore (Kia, VAG keys) was not loaded");
}

#[test]
fn replay_sends_the_recorded_pulses() {
    let mut req = request("replay", None, None);
    req.pulses = princeton();
    let plan = plan(&req).unwrap();
    assert_eq!(plan.pulses, princeton());
    assert!(!plan.fsk);
    assert_eq!(plan.modulation, "OOK");
    assert_eq!(plan.preset, "AM650");
    assert_eq!(plan.air_us, princeton().iter().map(|d| u64::from(d.unsigned_abs())).sum::<u64>());
    assert!(plan.sent.is_none() && plan.button.is_none());
}

#[test]
fn a_fixed_code_sends_itself_again() {
    let found = decode(&princeton(), u64::from(F433), false, false).unwrap();
    let d = &found.iter().find(|d| d.name == "Princeton").expect("Princeton decodes").decode;
    let actions = tx_actions(d);
    assert!(actions.encodes && actions.gate, "{actions:?}");
    let plan = plan(&request("next", Some(d), None)).unwrap();
    assert!(plan.sent.is_none(), "a fixed code keeps its capture");
    let again = decode(&plan.pulses, u64::from(F433), false, false).unwrap();
    let back = &again.iter().find(|d| d.name == "Princeton").expect("the transmission decodes").decode;
    assert_eq!((back.data.as_str(), back.bits), (d.data.as_str(), d.bits));
}

#[test]
fn the_car_menu_and_the_gate_menu_are_desktops() {
    let car = Decode {
        protocol: "Ford V0".into(),
        serial: Some(0x0012_3456),
        button: Some(1),
        counter: Some(5),
        crc_valid: true,
        data: "0".into(),
        bits: 80,
        encoder_capable: true,
        extra: None,
    };
    let actions = tx_actions(&car);
    assert!(actions.encodes && !actions.gate, "{actions:?}");
    let names: Vec<&str> = actions.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(names, ["Lock", "Unlock", "Trunk", "Panic"]);
    assert_eq!(actions.buttons.iter().map(|b| b.code).collect::<Vec<_>>(), [1, 2, 4, 8]);
    let locked = Decode { encoder_capable: false, ..car };
    let actions = tx_actions(&locked);
    assert!(!actions.encodes && actions.buttons.is_empty(), "no encoder without the key: {actions:?}");
    assert!(plan(&request("next", Some(&locked), None)).is_err());
}

/// Every remote KAT can create with its embedded keys ("Add manually": KeeLoq manufacturers,
/// Nice, CAME, FAAC, Security+, …) sends three presses; a rolling code's counter moves on each.
#[test]
fn manual_remotes_send_their_next_codes() {
    let remotes = manual_remotes(0x1234);
    assert!(remotes.len() >= 60, "only {} remote types", remotes.len());
    let (mut rolling, mut fixed, mut failed) = (0, 0, Vec::new());
    for (id, first, frequency) in &remotes {
        let actions = tx_actions(first);
        if !actions.encodes {
            failed.push(format!("{id}: no encoder ({})", first.protocol));
            continue;
        }
        let mut d = first.clone();
        let mut counters = Vec::new();
        for press in 0..3 {
            let mut req = request("next", Some(&d), None);
            req.frequency = *frequency;
            match plan(&req) {
                Ok(p) => {
                    assert!(!p.pulses.is_empty() && p.air_us > 0, "{id}");
                    assert!(tx_iq(&p.pulses, p.fsk, p.deviation_hz).is_ok(), "{id}: IQ");
                    if let Some(sent) = p.sent {
                        assert_eq!(sent.serial, d.serial, "{id}: the serial stays");
                        counters.push(sent.counter);
                        d = sent;
                    }
                }
                Err(e) => {
                    failed.push(format!("{id} press {}: {e}", press + 1));
                    break;
                }
            }
        }
        if counters.is_empty() {
            fixed += 1;
        } else {
            rolling += 1;
            assert_eq!(counters.len(), 3, "{id}: every press advances ({counters:?})");
            assert!(counters.windows(2).all(|w| w[0] != w[1]), "{id}: counters {counters:?}");
        }
    }
    println!("{} remote types: {rolling} rolling, {fixed} fixed", remotes.len());
    assert!(failed.is_empty(), "{} failed:\n{}", failed.len(), failed.join("\n"));
    assert!(rolling >= 30, "only {rolling} rolling codes");
}

#[test]
fn ook_iq_is_received_back_as_the_remote() {
    let mut req = request("replay", None, None);
    req.pulses = princeton();
    let plan = plan(&req).unwrap();
    let iq = tx_iq(&plan.pulses, false, 0.0).unwrap();
    assert_eq!(iq.len(), 4 * plan.air_us as usize, "2 samples a µs, 2 bytes a sample");
    let bursts = receive(&iq);
    let am: Vec<_> = bursts.iter().filter(|b| b.modulation == "AM").collect();
    let heard = am.iter().find(|b| {
        kat_rx::identify_receive(&b.pulses, u64::from(F433), false).unwrap().iter().any(|r| r.name == "Princeton")
    });
    let heard = heard.unwrap_or_else(|| panic!("Princeton not received back from {} bursts", bursts.len()));
    let highs: Vec<i32> = heard.pulses.iter().copied().filter(|d| *d > 0).collect();
    let close = highs.iter().filter(|d| (**d - 390).abs() <= 24 || (**d - 1170).abs() <= 24).count();
    assert!(close * 100 / highs.len() >= 95, "timing: {highs:?}");
}

#[test]
fn fsk_iq_is_received_back_with_its_timing() {
    // 2-FSK at ±47.6 kHz (FM476), 400 µs symbols: the FM slicer sees the same pulses.
    let mut pulses = Vec::new();
    for i in 0..120 {
        pulses.push(if i % 2 == 0 { 400 } else { -400 });
    }
    let iq = tx_iq(&pulses, true, 47_607.0).unwrap();
    let bursts = receive(&iq);
    let fm = bursts.iter().find(|b| b.modulation == "FM").unwrap_or_else(|| panic!("no FM burst in {}", bursts.len()));
    let symbols: Vec<i32> = fm.pulses.iter().map(|p| p.abs()).filter(|d| *d < 3000).collect();
    assert!(symbols.len() >= 100, "{} symbols", symbols.len());
    let close = symbols.iter().filter(|d| (**d - 400).abs() <= 24).count();
    assert!(close * 100 / symbols.len() >= 95, "FSK timing: {symbols:?}");
}

#[test]
fn iq_samples_are_exact() {
    let iq = tx_iq(&[500, -500], false, 0.0).unwrap();
    assert_eq!(iq.len(), 4000);
    assert!(iq[..2000].chunks(2).all(|s| (s[0] as i8 as i32).pow(2) + (s[1] as i8 as i32).pow(2) > 100 * 100), "carrier on");
    assert!(iq[2000..].iter().all(|&b| b == 0), "carrier off: exact zeros");
    let fsk = tx_iq(&[500, -500], true, 47_607.0).unwrap();
    assert!(fsk.chunks(2).all(|s| (s[0] as i8 as i32).pow(2) + (s[1] as i8 as i32).pow(2) > 100 * 100), "2-FSK: constant envelope");
}

#[test]
fn what_is_refused() {
    assert!(plan(&request("replay", None, None)).is_err(), "nothing to replay");
    assert!(plan(&request("next", None, None)).is_err(), "nothing to encode");
    assert!(plan(&request("fly", None, None)).is_err());
    let mut long = request("replay", None, None);
    long.pulses = vec![5_000_000, -5_000_000, 1_000_000];
    assert!(plan(&long).unwrap_err().contains("10 s"));
    assert!(tx_iq(&[0, 10], false, 0.0).is_err());
    assert!(tx_iq(&[100], true, f64::NAN).is_err());
}

#[test]
fn a_desktop_fob_brings_its_decode() {
    let text = r#"{
      "version": "2.0", "format": "kat-fob",
      "signal": {"protocol": "Princeton", "frequency": 433920000, "frequency_mhz": "433.920",
        "modulation": "OOK", "encryption": "None", "data_bits": 24, "data_hex": "A5965A",
        "serial": "000A5965", "key": "A5965A", "button": 10, "button_name": "-", "counter": null,
        "crc_valid": false, "encoder_capable": true, "data_extra": "00000064000001A3"},
      "vehicle": {"make": ""},
      "capture": {"timestamp": "2026-11-26T12:00:00Z", "raw_data_hex": "A5965A", "raw_pair_count": 0}
    }"#;
    let d = kat_rx::mobile_tx::fob_decode(text).expect("decode from the .fob");
    assert_eq!((d.protocol.as_str(), d.data.as_str(), d.bits, d.encoder_capable), ("Princeton", "A5965A", 24, true));
    assert_eq!(d.extra.as_deref(), Some("64000001A3"));
}

/// The app's JSON reader takes no `null` for an object: absent fields are left out.
#[test]
fn absent_fields_are_left_out_of_the_json() {
    let mut req = request("replay", None, None);
    req.pulses = princeton();
    let json = serde_json::to_value(plan(&req).unwrap()).unwrap();
    let fields = json.as_object().unwrap();
    for absent in ["sent", "button", "button_label"] {
        assert!(!fields.contains_key(absent), "{absent} in {json}");
    }
}
