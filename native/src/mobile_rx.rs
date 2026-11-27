//! JNI ownership wrapper around the unmodified desktop KAT receive DSP.
use crate::radio::{pipeline::{RxPipeline,Burst},dsp::IqFormat};
use serde::Serialize;
use std::{collections::HashMap,sync::{Mutex,OnceLock,atomic::{AtomicI64,Ordering}}};
use jni::{JNIEnv,objects::{JClass,JByteArray},sys::{jstring,jlong,jint,jboolean,jdouble}};

pub struct ReceiveSession {pipeline:RxPipeline}
#[derive(Serialize)]
pub struct ReceiveBurst {pub modulation:String,pub pulses:Vec<i32>,pub snr_db:f32,pub offset_hz:f32}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct ReceiveOutput {pub active:bool,pub power_db:f32,pub bursts:Vec<ReceiveBurst>}
impl ReceiveSession {
    pub fn new(rate:u32,frequency:u32)->Self{Self{pipeline:RxPipeline::new(rate,0.0,frequency)}}
    pub fn with_offset(rate:u32,frequency:u32,offset:f64)->Self{Self{pipeline:RxPipeline::new(rate,offset,frequency)}}
    pub fn process(&mut self,bytes:&[u8])->ReceiveOutput{let bursts=self.pipeline.process(bytes,IqFormat::Cs8);self.output(bursts)}
    pub fn flush(&mut self)->ReceiveOutput{let bursts=self.pipeline.flush();self.output(bursts)}
    fn output(&mut self,bursts:Vec<Burst>)->ReceiveOutput{
        let power_db=(20.0*self.pipeline.take_rssi().max(0.00001).log10()).clamp(-100.0,6.0);
        let active=self.pipeline.busy();let mut out=Vec::new();
        for b in bursts {
            for (mode,pairs) in [("AM",b.am),("FM",b.fm)] {
                if let Some(pairs)=pairs {
                    if pairs.len()>32768{continue}
                    let pulses=pairs.into_iter().map(|p|if p.level{p.duration_us.min(i32::MAX as u32) as i32}else{-(p.duration_us.min(i32::MAX as u32) as i32)}).collect();
                    out.push(ReceiveBurst{modulation:mode.into(),pulses,snr_db:b.snr_db,offset_hz:b.offset_hz});
                }
            }
        }
        ReceiveOutput{active,power_db,bursts:out}
    }
}
static SESSIONS:OnceLock<Mutex<HashMap<i64,ReceiveSession>>>=OnceLock::new();
static NEXT:AtomicI64=AtomicI64::new(1);
fn sessions()->&'static Mutex<HashMap<i64,ReceiveSession>>{SESSIONS.get_or_init(||Mutex::new(HashMap::new()))}
#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRx_nativeCreate(mut env:JNIEnv,_class:JClass,rate:jint,frequency:jlong,offset:jdouble)->jlong{
    let result=std::panic::catch_unwind(||{
        if !(250_000..=20_000_000).contains(&rate)||frequency<=0||!offset.is_finite()||offset.abs()>rate as f64/2.0{return Err("Invalid RX settings")}
        let mut map=sessions().lock().unwrap_or_else(|e|e.into_inner());
        if map.len()>=16{return Err("Too many RX sessions")}
        let id=NEXT.fetch_add(1,Ordering::Relaxed);
        map.insert(id,ReceiveSession::with_offset(rate as u32,frequency.min(u32::MAX as i64) as u32,offset));Ok(id)
    });
    match result{Ok(Ok(id))=>id,_=>{let _=env.throw_new("java/lang/IllegalStateException","KAT RX initialization failed");0}}
}
#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRx_nativeProcess(mut env:JNIEnv,_class:JClass,id:jlong,input:JByteArray,length:jint,flush:jboolean)->jstring{
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<String,String>{
        if !(0..=2_097_152).contains(&length){return Err("Invalid IQ block length".into())}
        if env.get_array_length(&input).map_err(|e|e.to_string())?<length{return Err("Truncated IQ block".into())}
        let mut signed=vec![0i8;length as usize];env.get_byte_array_region(&input,0,&mut signed).map_err(|e|e.to_string())?;
        let bytes:Vec<u8>=signed.into_iter().map(|b|b as u8).collect();
        let mut map=sessions().lock().unwrap_or_else(|e|e.into_inner());
        let rx=map.get_mut(&id).ok_or("RX session is closed")?;
        let out=if flush!=0{rx.flush()}else{rx.process(&bytes)};
        serde_json::to_string(&out).map_err(|e|e.to_string())
    }));
    match result {
        Ok(Ok(json))=>env.new_string(json).map(|s|s.into_raw()).unwrap_or(std::ptr::null_mut()),
        _=>{let _=env.throw_new("java/lang/IllegalStateException","KAT RX processing failed");std::ptr::null_mut()}
    }
}
#[no_mangle]
pub extern "system" fn Java_app_katmobile_NativeRx_nativeClose(_env:JNIEnv,_class:JClass,id:jlong){sessions().lock().unwrap_or_else(|e|e.into_inner()).remove(&id);}
