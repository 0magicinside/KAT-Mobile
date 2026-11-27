use std::sync::{Arc, Mutex};
use std::time::Duration;

use kat_rx::rtlsdr::device::{
    BLOCK_IIC, BLOCK_SYS, BLOCK_USB, CTRL_IN, CTRL_OUT, DEMOD_CTL, EEPROM_ADDR, GPO, USB_EPA_CTL,
};
use kat_rx::rtlsdr::{is_known_device, Result, RtlSdr, RtlsdrError, TunerGain, UsbTransport};

#[derive(Debug, Clone)]
struct ControlOp {
    input: bool,
    value: u16,
    index: u16,
    data: Vec<u8>,
}

#[derive(Debug)]
struct MockRtlDongle {
    tuner_i2c_addr: Option<u16>,
    fail_eeprom: bool,
    eeprom_byte7: u8,
    usb_strings: (Option<String>, Option<String>, Option<String>),
    ops: Arc<Mutex<Vec<ControlOp>>>,
    last_i2c_reg: Mutex<u8>,
    gpo: Mutex<u8>,
}

impl MockRtlDongle {
    fn r820t() -> (Self, Arc<Mutex<Vec<ControlOp>>>) {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let dongle = Self {
            tuner_i2c_addr: Some(0x34),
            fail_eeprom: false,
            eeprom_byte7: 0x16, // Realtek profile: neither bias-tee nor direct-sampling forced
            usb_strings: (
                Some("Realtek".into()),
                Some("RTL2838UHIDIR".into()),
                Some("00000001".into()),
            ),
            ops: Arc::clone(&ops),
            last_i2c_reg: Mutex::new(0),
            gpo: Mutex::new(0),
        };
        (dongle, ops)
    }

    fn blog_v4() -> (Self, Arc<Mutex<Vec<ControlOp>>>) {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let dongle = Self {
            tuner_i2c_addr: Some(0x74), // R828D I2C address
            fail_eeprom: false,
            eeprom_byte7: 0x16,
            usb_strings: (
                Some("RTLSDRBlog".into()),
                Some("Blog V4".into()),
                Some("00000001".into()),
            ),
            ops: Arc::clone(&ops),
            last_i2c_reg: Mutex::new(0),
            gpo: Mutex::new(0),
        };
        (dongle, ops)
    }
}

impl UsbTransport for MockRtlDongle {
    fn read_control(
        &self,
        request_type: u8,
        _request: u8,
        value: u16,
        index: u16,
        buf: &mut [u8],
        _timeout: Duration,
    ) -> Result<usize> {
        assert_eq!(request_type, CTRL_IN);
        let block = index >> 8;
        if block == BLOCK_IIC && value == EEPROM_ADDR {
            if self.fail_eeprom {
                return Err(RtlsdrError::Usb("EEPROM Pipe error".into()));
            }
            for b in buf.iter_mut() {
                *b = self.eeprom_byte7;
            }
        } else if block == BLOCK_IIC && Some(value) == self.tuner_i2c_addr {
            // The tuner probe (`i2c_read_reg`: one byte from register 0) reads the raw 0x69 an
            // R82xx answers with. R82xx::read_reg bit-reverses what it reads and checks the PLL
            // lock as data[2] & 0x40: bit_reverse(0x02) == 0x40, so every other read says locked.
            let reg = *self.last_i2c_reg.lock().unwrap();
            if buf.len() == 1 && reg == 0x00 {
                // Tuner probe (`i2c_read_reg`) expects 0x69 without bit-reversal.
                buf[0] = 0x69;
            } else {
                for b in buf.iter_mut() {
                    *b = 0x02; // bit_reverse(0x02) = 0x40 -> PLL locked
                }
            }
        } else if block == BLOCK_IIC {
            return Err(RtlsdrError::Usb(format!("No I2C ACK at 0x{value:02x}")));
        } else if block == BLOCK_SYS && value == GPO {
            if !buf.is_empty() {
                buf[0] = *self.gpo.lock().unwrap();
            }
        } else {
            buf.fill(0);
        }
        self.ops.lock().unwrap().push(ControlOp {
            input: true,
            value,
            index,
            data: buf.to_vec(),
        });
        Ok(buf.len())
    }

    fn write_control(
        &self,
        request_type: u8,
        _request: u8,
        value: u16,
        index: u16,
        buf: &[u8],
        _timeout: Duration,
    ) -> Result<usize> {
        assert_eq!(request_type, CTRL_OUT);
        let block = index >> 8;
        if block == BLOCK_IIC && Some(value) == self.tuner_i2c_addr && !buf.is_empty() {
            *self.last_i2c_reg.lock().unwrap() = buf[0];
        } else if block == BLOCK_IIC && value != EEPROM_ADDR {
            return Err(RtlsdrError::Usb(format!("No I2C ACK at 0x{value:02x}")));
        } else if block == BLOCK_SYS && value == GPO && !buf.is_empty() {
            *self.gpo.lock().unwrap() = buf[0];
        }
        self.ops.lock().unwrap().push(ControlOp {
            input: false,
            value,
            index,
            data: buf.to_vec(),
        });
        Ok(buf.len())
    }

    fn usb_strings(&self) -> Result<(Option<String>, Option<String>, Option<String>)> {
        Ok(self.usb_strings.clone())
    }
}

#[test]
fn r820t_open_configure_tune_gain_ppm_biastee_and_close() {
    let (dongle, ops) = MockRtlDongle::r820t();
    let mut sdr = RtlSdr::open(Box::new(dongle)).expect("R820T open should succeed");

    assert_eq!(sdr.get_tuner_id().unwrap(), "r820t");
    assert!(!sdr.is_blog_v4());
    let gains = sdr.get_tuner_gains().unwrap();
    assert_eq!(gains.len(), 29);
    assert_eq!(*gains.first().unwrap(), 0);
    assert_eq!(*gains.last().unwrap(), 496);

    sdr.reset_buffer().unwrap();
    sdr.set_freq_correction(-12).unwrap();
    assert_eq!(sdr.get_freq_correction(), -12);
    sdr.set_center_freq(433_670_000).unwrap();
    assert_eq!(sdr.get_center_freq(), 433_670_000);
    sdr.set_sample_rate(2_000_000).unwrap();
    assert_eq!(sdr.get_sample_rate(), 2_000_000);
    sdr.set_tuner_gain(TunerGain::Manual(402)).unwrap();
    sdr.set_tuner_gain(TunerGain::Auto).unwrap();
    sdr.set_bias_tee(true).unwrap();

    // Check buffer reset and bias-tee GPO writes happened
    {
        let recorded = ops.lock().unwrap();
        assert!(recorded.iter().any(|op| !op.input
            && op.value == USB_EPA_CTL
            && (op.index >> 8) == BLOCK_USB
            && op.data == vec![0x10, 0x02]));
        assert!(recorded.iter().any(|op| !op.input
            && op.value == GPO
            && (op.index >> 8) == BLOCK_SYS
            && op.data == vec![0x01]));
    }

    sdr.close().unwrap();
    let recorded = ops.lock().unwrap();
    // Deinit must disable bias-tee (GPO bit 0 = 0) and power down demodulator (DEMOD_CTL = 0x20).
    let last_gpo = recorded
        .iter()
        .rev()
        .find(|op| !op.input && op.value == GPO && (op.index >> 8) == BLOCK_SYS)
        .expect("GPO write on close");
    assert_eq!(last_gpo.data[0] & 0x01, 0);
    let last_demod = recorded
        .iter()
        .rev()
        .find(|op| !op.input && op.value == DEMOD_CTL && (op.index >> 8) == BLOCK_SYS)
        .expect("DEMOD_CTL write on close");
    assert_eq!(last_demod.data, vec![0x20]);
}

#[test]
fn r828d_blog_v4_detected_from_usb_strings_and_tunes_hf_and_uhf() {
    let (dongle, _ops) = MockRtlDongle::blog_v4();
    let mut sdr = RtlSdr::open(Box::new(dongle)).expect("Blog V4 open should succeed");

    assert_eq!(sdr.get_tuner_id().unwrap(), "r828d");
    assert!(sdr.is_blog_v4());
    assert!(sdr.get_tuner_name().unwrap().contains("Blog V4"));

    sdr.set_sample_rate(2_000_000).unwrap();
    // Standard keyfob frequency
    sdr.set_center_freq(433_670_000).unwrap();
    assert_eq!(sdr.get_center_freq(), 433_670_000);
    // HF frequency handled by Blog V4 built-in 28.8 MHz upconverter
    sdr.set_center_freq(10_000_000).unwrap();
    assert_eq!(sdr.get_center_freq(), 10_000_000);
    sdr.close().unwrap();
}

#[test]
fn unknown_tuner_returns_error_instead_of_panicking() {
    let (mut dongle, _) = MockRtlDongle::r820t();
    dongle.tuner_i2c_addr = None;
    let err = RtlSdr::open(Box::new(dongle)).expect_err("Missing tuner must fail");
    assert!(matches!(err, RtlsdrError::UnknownTuner));
    assert!(err.to_string().contains("R820T"));
}

#[test]
fn failed_eeprom_read_does_not_fail_open_or_force_bias_tee() {
    let (mut dongle, ops) = MockRtlDongle::r820t();
    dongle.fail_eeprom = true;
    let mut sdr = RtlSdr::open(Box::new(dongle)).expect("Dongle without readable EEPROM must still open");
    sdr.set_bias_tee(false).unwrap();
    let last_gpo = ops
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|op| !op.input && op.value == GPO && (op.index >> 8) == BLOCK_SYS)
        .cloned()
        .expect("GPO write");
    assert_eq!(last_gpo.data[0] & 0x01, 0);
    sdr.close().unwrap();
}

#[test]
fn invalid_sample_rate_is_rejected() {
    let (dongle, _) = MockRtlDongle::r820t();
    let mut sdr = RtlSdr::open(Box::new(dongle)).unwrap();
    assert!(sdr.set_sample_rate(20_000_000).is_err());
    assert!(sdr.set_sample_rate(500_000).is_err());
    assert!(sdr.set_sample_rate(100_000).is_err());
}

#[test]
fn known_vid_pid_table_covers_rtl_dongles_and_excludes_hackrf() {
    assert!(is_known_device(0x0bda, 0x2832));
    assert!(is_known_device(0x0bda, 0x2838));
    assert!(is_known_device(0x1f4d, 0xd803));
    assert!(!is_known_device(0x1d50, 0x6089)); // HackRF One
}
