use kat_rx::{ReceiveSession,identify};
use std::f64::consts::TAU;

fn princeton_iq(offset:f64,noise:f64)->Vec<u8>{
    let mut raw=Vec::new();let mut sample=0u64;let mut rng=0x17a52e31u32;
    let mut segment=|us:usize,on:bool|{
        for _ in 0..us*2{
            let phase=TAU*offset*sample as f64/2_000_000.0;sample+=1;
            for (tone,dc) in [(phase.cos(),4.0),(phase.sin(),-3.0)]{
                rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let n=((rng>>8) as f64/16_777_216.0-0.5)*2.0*noise;
                let x=(dc+n+if on{tone*65.0}else{0.0}).round().clamp(-127.0,127.0) as i8;
                raw.push(x as u8);
            }
        }
    };
    segment(30000,false);
    for _ in 0..4{
        for c in "101001011001011010010110".chars(){segment(if c=='1'{1170}else{390},true);segment(if c=='1'{390}else{1170},false);}
        segment(390,true);segment(11700,false);
    }
    segment(100000,false);raw
}

#[test]fn desktop_pipeline_recognizes_rotating_noisy_princeton_and_preserves_timing(){
    for offset in [-250000.0,-110000.0,0.0,150000.0,250000.0]{
        let mut rx=ReceiveSession::new(2_000_000,433_920_000);let raw=princeton_iq(offset,6.0);let mut bursts=Vec::new();
        // Odd block boundaries also exercise the desktop pipeline's IQ carry.
        for chunk in raw.chunks(16383){bursts.extend(rx.process(chunk).bursts)}
        bursts.extend(rx.flush().bursts);
        let mut found=false;
        for b in bursts.iter().filter(|b|b.modulation=="AM"){
            if identify(&b.pulses,433_920_000,false).unwrap().iter().any(|r|r.name=="Princeton"){
                found=true;
                let highs:Vec<i32>=b.pulses.iter().copied().filter(|d|*d>0).collect();
                assert!(highs.len()>=24);
                assert!(highs.iter().filter(|d|(**d-390).abs()<=24||(**d-1170).abs()<=24).count()*100/highs.len()>=95,"Timing distorted at offset {offset}: {highs:?}");
            }
        }
        assert!(found,"Princeton not recognized at {offset} Hz; bursts {}",bursts.len());
    }
}

#[test]fn empty_input_is_not_a_radio_signal(){let mut rx=ReceiveSession::new(2_000_000,433_920_000);for _ in 0..30{assert!(rx.process(&vec![0;16384]).bursts.is_empty())}assert!(rx.flush().bursts.is_empty());}

#[test]fn live_offset_tuning_matches_desktop_target_and_preserves_led_frames(){
    let mut rx=ReceiveSession::with_offset(2_000_000,433_920_000,250_000.0);
    let raw=princeton_iq(225_000.0,6.0);let mut bursts=Vec::new();
    for c in raw.chunks(65536){bursts.extend(rx.process(c).bursts)}bursts.extend(rx.flush().bursts);
    assert!(bursts.iter().any(|b|b.modulation=="AM"&&(b.offset_hz+25000.0).abs()<2000.0&&kat_rx::identify_receive(&b.pulses,433_920_000,false).unwrap().iter().any(|r|r.name=="Princeton")));
}

#[test]fn desktop_fsk_slicer_tracks_both_tones_away_from_channel_center(){
    let mut raw=Vec::new();let mut phase:f64=0.0;let mut rng=19u32;
    for n in 0..300000usize{
        let on=(60000..140000).contains(&n);
        let tone=if (n.saturating_sub(60000)/800)%2==0{30000.0}else{-30000.0};
        phase+=TAU*(150000.0+tone)/2_000_000.0;
        for v in [phase.cos(),phase.sin()]{rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);let noise=((rng>>8) as f64/16777216.0-0.5)*8.0;raw.push((noise+if on{v*65.0}else{0.0}).round() as i8 as u8);}
    }
    let mut rx=ReceiveSession::new(2_000_000,433_920_000);let mut bursts=Vec::new();
    for c in raw.chunks(32768){bursts.extend(rx.process(c).bursts)}bursts.extend(rx.flush().bursts);
    let fm=bursts.iter().find(|b|b.modulation=="FM").expect("No FM burst");
    assert!(fm.pulses.len()>=90);
    let symbols:Vec<_>=fm.pulses.iter().map(|p|p.abs()).filter(|d|*d<3000).collect();
    assert!(symbols.iter().filter(|d|(**d-400).abs()<=24).count()*100/symbols.len()>=95,"FSK timings: {symbols:?}");
}

#[test]fn cu8_rtlsdr_stream_with_leading_zero_bytes_and_offset_tuning_decodes_princeton(){
    let cs8=princeton_iq(225_000.0,6.0);
    let mut cu8=vec![0u8;20_000]; // RTL-SDR dead startup zero bytes (~5 ms at 2 MS/s)
    cu8.extend(cs8.into_iter().map(|b|((b as i8 as i16)+128).clamp(0,255) as u8));
    let mut rx=ReceiveSession::with_format(2_000_000,433_920_000,250_000.0,kat_rx::IqFormat::Cu8);
    let mut bursts=Vec::new();
    for c in cu8.chunks(16384){bursts.extend(rx.process(c).bursts)}
    bursts.extend(rx.flush().bursts);
    assert!(bursts.iter().any(|b|b.modulation=="AM"&&(b.offset_hz+25_000.0).abs()<2000.0&&kat_rx::identify_receive(&b.pulses,433_920_000,false).unwrap().iter().any(|r|r.name=="Princeton")));
}

#[test]fn cu8_rtlsdr_fsk_slicer_tracks_both_tones(){
    let mut cu8=Vec::new();let mut phase:f64=0.0;let mut rng=29u32;
    for n in 0..300000usize{
        let on=(60000..140000).contains(&n);
        let tone=if (n.saturating_sub(60000)/800)%2==0{30000.0}else{-30000.0};
        phase+=TAU*(150000.0+tone)/2_000_000.0;
        for v in [phase.cos(),phase.sin()]{rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);let noise=((rng>>8) as f64/16777216.0-0.5)*8.0;let sample=(127.5+noise+if on{v*65.0}else{0.0}).round().clamp(0.0,255.0) as u8;cu8.push(sample);}
    }
    let mut rx=ReceiveSession::with_format(2_000_000,433_920_000,0.0,kat_rx::IqFormat::Cu8);
    let mut bursts=Vec::new();
    for c in cu8.chunks(32768){bursts.extend(rx.process(c).bursts)}
    bursts.extend(rx.flush().bursts);
    let fm=bursts.iter().find(|b|b.modulation=="FM").expect("No CU8 FM burst");
    assert!(fm.pulses.len()>=90);
    let symbols:Vec<_>=fm.pulses.iter().map(|p|p.abs()).filter(|d|*d<3000).collect();
    assert!(symbols.iter().filter(|d|(**d-400).abs()<=24).count()*100/symbols.len()>=95,"CU8 FSK timings: {symbols:?}");
}
