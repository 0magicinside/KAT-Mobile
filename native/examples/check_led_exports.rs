//! Local, passive diagnostic for explicitly supplied Princeton RAW recordings.
use std::{env,fs,path::Path};
fn main(){
    let dir=env::args().nth(1).expect("Directory containing authorized led-0..3.sub exports");
    for n in 0..4 {
        let path=Path::new(&dir).join(format!("led-{n}.sub"));
        let text=fs::read_to_string(&path).expect("Read LED export");
        let durations:Vec<i32>=text.lines().filter_map(|s|s.strip_prefix("RAW_Data:")).flat_map(str::split_whitespace).map(|s|s.parse().expect("Duration")).collect();
        let found=kat_rx::identify(&durations,433_920_000,false).expect("Decode exported pulses");
        assert!(found.iter().any(|r|r.name=="Princeton"&&r.bits==24),"Export {n} failed receive decoding");
        let fob_path=Path::new(&dir).join("kat-fob").join(format!("led-{n}.fob"));
        let fob=kat_rx::interop_fob::parse_fob(&fs::read_to_string(fob_path).expect("Read LED FOB"),1).expect("Original KAT FOB parser");
        let saved:Vec<i32>=fob.raw_pairs.iter().map(|p|if p.level{p.duration_us as i32}else{-(p.duration_us as i32)}).collect();
        assert_eq!(saved,durations,"FOB changed stored pulse timings");
        assert!(kat_rx::identify_receive(&saved,fob.frequency as u64,false).unwrap().iter().any(|r|r.name=="Princeton"));
        println!("LED export {n}: {} pulses preserved exactly in RAW SUB and KAT FOB; original KAT parser and Princeton receive decoder accept both",durations.len());
    }
}
