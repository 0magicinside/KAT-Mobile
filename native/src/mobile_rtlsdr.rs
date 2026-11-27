//! JNI bridge between Kotlin's Android USB Host control transfers (`RtlSdrUsb.kt`)
//! and the embedded MPL-2.0 RTL-SDR control driver (`crate::rtlsdr`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jintArray, jlong, jstring};
use jni::{JNIEnv, JavaVM};

use crate::rtlsdr::{Result as RtlResult, RtlSdr, RtlsdrError, TunerGain, UsbTransport};

struct JniUsbTransport {
    vm: JavaVM,
    bridge: GlobalRef,
    manufacturer: Option<String>,
    product: Option<String>,
    serial: Option<String>,
}

impl std::fmt::Debug for JniUsbTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JniUsbTransport")
            .field("manufacturer", &self.manufacturer)
            .field("product", &self.product)
            .field("serial", &self.serial)
            .finish()
    }
}

impl UsbTransport for JniUsbTransport {
    fn read_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buf: &mut [u8],
        timeout: Duration,
    ) -> RtlResult<usize> {
        let mut env = self
            .vm
            .get_env()
            .map_err(|e| RtlsdrError::Usb(format!("JNI environment unavailable: {e}")))?;
        let timeout_ms = timeout.as_millis().clamp(1, i32::MAX as u128) as jint;
        let len = buf.len() as jint;
        let result = env.with_local_frame(16, |env| -> std::result::Result<RtlResult<usize>, jni::errors::Error> {
            let arr = env.new_byte_array(len)?;
            let ret = env
                .call_method(
                    self.bridge.as_obj(),
                    "controlTransfer",
                    "(IIII[BII)I",
                    &[
                        JValue::Int(request_type as jint),
                        JValue::Int(request as jint),
                        JValue::Int(value as jint),
                        JValue::Int(index as jint),
                        JValue::Object(&arr),
                        JValue::Int(len),
                        JValue::Int(timeout_ms),
                    ],
                )?
                .i()?;
            if ret < 0 {
                return Ok(Err(RtlsdrError::Usb(format!(
                    "USB control IN failed (addr=0x{value:04x}, idx=0x{index:04x}, ret={ret})"
                ))));
            }
            let count = (ret as usize).min(buf.len());
            if count > 0 {
                let mut signed = vec![0i8; count];
                env.get_byte_array_region(&arr, 0, &mut signed)?;
                for (dst, src) in buf[..count].iter_mut().zip(signed) {
                    *dst = src as u8;
                }
            }
            Ok(Ok(count))
        });
        match result {
            Ok(inner) => inner,
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                Err(RtlsdrError::Usb(format!("JNI USB control IN error: {e}")))
            }
        }
    }

    fn write_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buf: &[u8],
        timeout: Duration,
    ) -> RtlResult<usize> {
        let mut env = self
            .vm
            .get_env()
            .map_err(|e| RtlsdrError::Usb(format!("JNI environment unavailable: {e}")))?;
        let timeout_ms = timeout.as_millis().clamp(1, i32::MAX as u128) as jint;
        let len = buf.len() as jint;
        let result = env.with_local_frame(16, |env| -> std::result::Result<RtlResult<usize>, jni::errors::Error> {
            let arr = env.new_byte_array(len)?;
            if !buf.is_empty() {
                let signed: Vec<i8> = buf.iter().map(|&b| b as i8).collect();
                env.set_byte_array_region(&arr, 0, &signed)?;
            }
            let ret = env
                .call_method(
                    self.bridge.as_obj(),
                    "controlTransfer",
                    "(IIII[BII)I",
                    &[
                        JValue::Int(request_type as jint),
                        JValue::Int(request as jint),
                        JValue::Int(value as jint),
                        JValue::Int(index as jint),
                        JValue::Object(&arr),
                        JValue::Int(len),
                        JValue::Int(timeout_ms),
                    ],
                )?
                .i()?;
            if ret < 0 {
                return Ok(Err(RtlsdrError::Usb(format!(
                    "USB control OUT failed (addr=0x{value:04x}, idx=0x{index:04x}, ret={ret})"
                ))));
            }
            Ok(Ok(ret as usize))
        });
        match result {
            Ok(inner) => inner,
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                Err(RtlsdrError::Usb(format!("JNI USB control OUT error: {e}")))
            }
        }
    }

    fn reset(&self) -> RtlResult<()> {
        let mut env = self
            .vm
            .get_env()
            .map_err(|e| RtlsdrError::Usb(format!("JNI environment unavailable: {e}")))?;
        let res = env.with_local_frame(8, |env| {
            env.call_method(self.bridge.as_obj(), "resetInterface", "()Z", &[])?.z()
        });
        match res {
            Ok(true) => Ok(()),
            Ok(false) => Err(RtlsdrError::Usb("Could not reset USB interface".into())),
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                Err(RtlsdrError::Usb(format!("JNI USB reset error: {e}")))
            }
        }
    }

    fn usb_strings(&self) -> RtlResult<(Option<String>, Option<String>, Option<String>)> {
        Ok((
            self.manufacturer.clone(),
            self.product.clone(),
            self.serial.clone(),
        ))
    }
}

static RTL_SESSIONS: OnceLock<Mutex<HashMap<i64, RtlSdr>>> = OnceLock::new();
static NEXT_RTL: AtomicI64 = AtomicI64::new(1);

fn rtl_sessions() -> &'static Mutex<HashMap<i64, RtlSdr>> {
    RTL_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_opt_string(env: &mut JNIEnv, s: JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(&s)
        .ok()
        .map(|js| js.to_string_lossy().trim_end_matches('\0').to_string())
        .filter(|v| !v.is_empty())
}

fn validate_center_freq(sdr: &RtlSdr, center_hz: i64) -> std::result::Result<u32, String> {
    let min_hz = if sdr.is_blog_v4() { 500_000 } else { 24_000_000 };
    let max_hz = 1_766_000_000;
    if !(min_hz..=max_hz).contains(&center_hz) {
        return Err(format!(
            "Frequency {:.3} MHz is outside the RTL-SDR tuner range ({:.1}–{:.0} MHz)",
            center_hz as f64 / 1e6,
            min_hz as f64 / 1e6,
            max_hz as f64 / 1e6,
        ));
    }
    Ok(center_hz as u32)
}

fn throw_state(env: &mut JNIEnv, message: &str) {
    if !env.exception_check().unwrap_or(false) {
        let _ = env.throw_new("java/lang/IllegalStateException", message);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeOpen(
    mut env: JNIEnv,
    _class: JClass,
    bridge: JObject,
    manufacturer: JString,
    product: JString,
    serial: JString,
) -> jlong {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<i64, String> {
        let vm = env.get_java_vm().map_err(|e| format!("Could not acquire JavaVM: {e}"))?;
        let global = env
            .new_global_ref(bridge)
            .map_err(|e| format!("Could not retain USB bridge: {e}"))?;
        let mfg = read_opt_string(&mut env, manufacturer);
        let prod = read_opt_string(&mut env, product);
        let ser = read_opt_string(&mut env, serial);
        let transport = Box::new(JniUsbTransport {
            vm,
            bridge: global,
            manufacturer: mfg,
            product: prod,
            serial: ser,
        });
        let sdr = RtlSdr::open(transport).map_err(|e| e.to_string())?;
        let mut map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= 8 {
            return Err("Too many open RTL-SDR sessions".into());
        }
        let id = NEXT_RTL.fetch_add(1, Ordering::Relaxed);
        map.insert(id, sdr);
        Ok(id)
    }));
    match result {
        Ok(Ok(id)) => id,
        Ok(Err(msg)) => {
            throw_state(&mut env, &msg);
            0
        }
        Err(_) => {
            throw_state(&mut env, "RTL-SDR initialization panicked");
            0
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeTunerName(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<String, String> {
        let map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        let sdr = map.get(&handle).ok_or("RTL-SDR session is closed")?;
        sdr.get_tuner_name().map_err(|e| e.to_string())
    }));
    match result {
        Ok(Ok(name)) => env
            .new_string(name)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Ok(Err(msg)) => {
            throw_state(&mut env, &msg);
            std::ptr::null_mut()
        }
        Err(_) => {
            throw_state(&mut env, "RTL-SDR tuner lookup failed");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeTunerGains(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jintArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<Vec<i32>, String> {
        let map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        let sdr = map.get(&handle).ok_or("RTL-SDR session is closed")?;
        sdr.get_tuner_gains().map_err(|e| e.to_string())
    }));
    match result {
        Ok(Ok(gains)) => match env.new_int_array(gains.len() as i32) {
            Ok(arr) => {
                if env.set_int_array_region(&arr, 0, &gains).is_ok() {
                    arr.into_raw()
                } else {
                    throw_state(&mut env, "Could not copy RTL-SDR gains");
                    std::ptr::null_mut()
                }
            }
            Err(_) => {
                throw_state(&mut env, "Could not allocate RTL-SDR gains array");
                std::ptr::null_mut()
            }
        },
        Ok(Err(msg)) => {
            throw_state(&mut env, &msg);
            std::ptr::null_mut()
        }
        Err(_) => {
            throw_state(&mut env, "RTL-SDR gains lookup failed");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeConfigure(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    center_hz: jlong,
    sample_rate: jint,
    auto_gain: jboolean,
    gain_tenths_db: jint,
    ppm: jint,
    bias_tee: jboolean,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<(), String> {
        if !(225_001..=3_200_000).contains(&sample_rate) {
            return Err(format!("Unsupported RTL-SDR sample rate: {sample_rate} S/s"));
        }
        let mut map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        let sdr = map.get_mut(&handle).ok_or("RTL-SDR session is closed")?;
        let freq = validate_center_freq(sdr, center_hz)?;
        sdr.reset_buffer().map_err(|e| format!("RTL-SDR buffer reset failed: {e}"))?;
        if ppm != 0 {
            sdr.set_freq_correction(ppm)
                .map_err(|e| format!("RTL-SDR PPM correction failed: {e}"))?;
        }
        sdr.set_center_freq(freq)
            .map_err(|e| format!("RTL-SDR frequency tune failed: {e}"))?;
        sdr.set_sample_rate(sample_rate as u32)
            .map_err(|e| format!("RTL-SDR sample rate setup failed: {e}"))?;
        sdr.set_bias_tee(bias_tee != 0)
            .map_err(|e| format!("RTL-SDR bias-tee setup failed: {e}"))?;
        let gain = if auto_gain != 0 {
            TunerGain::Auto
        } else {
            TunerGain::Manual(gain_tenths_db)
        };
        sdr.set_tuner_gain(gain)
            .map_err(|e| format!("RTL-SDR gain setup failed: {e}"))?;
        sdr.reset_buffer().map_err(|e| format!("RTL-SDR buffer reset failed: {e}"))?;
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => throw_state(&mut env, &msg),
        Err(_) => throw_state(&mut env, "RTL-SDR configuration failed"),
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeSetCenterFreq(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    center_hz: jlong,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<(), String> {
        let mut map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        let sdr = map.get_mut(&handle).ok_or("RTL-SDR session is closed")?;
        let freq = validate_center_freq(sdr, center_hz)?;
        sdr.set_center_freq(freq)
            .map_err(|e| format!("RTL-SDR retune failed: {e}"))
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => throw_state(&mut env, &msg),
        Err(_) => throw_state(&mut env, "RTL-SDR retune panicked"),
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeResetBuffer(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> std::result::Result<(), String> {
        let map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        let sdr = map.get(&handle).ok_or("RTL-SDR session is closed")?;
        sdr.reset_buffer().map_err(|e| e.to_string())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => throw_state(&mut env, &msg),
        Err(_) => throw_state(&mut env, "RTL-SDR buffer reset panicked"),
    }
}

#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRtlSdr_nativeClose(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let sdr = {
        let mut map = rtl_sessions().lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&handle)
    };
    if let Some(mut sdr) = sdr {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = sdr.close();
        }));
    }
}
