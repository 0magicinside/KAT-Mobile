use std::{fs,path::Path};
#[test]fn android_export_passes_the_original_desktop_kat_parser(){
    let path=Path::new("../app/build/interop-tests/export.fob");
    let text=fs::read_to_string(path).expect("Run Gradle JVM tests first to create the Android FOB fixture");
    let capture=kat_rx::interop_fob::parse_fob(&text,1).expect("Original desktop KAT parser rejected Android FOB");
    assert_eq!(capture.frequency,868350000);
    assert_eq!(capture.protocol.as_deref(),Some("Synthetic test"));
    assert_eq!(format!("{:?}",capture.received_rf),"Some(FM)");
    assert_eq!(capture.raw_pairs.len(),170);
    for (i,p) in capture.raw_pairs.iter().enumerate(){assert_eq!(p.level,i%2==1);assert_eq!(p.duration_us,200+i as u32*3);}
}
