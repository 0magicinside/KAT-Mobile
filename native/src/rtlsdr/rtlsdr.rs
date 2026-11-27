// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use tracing::{error, info, warn};

use super::device::{
    Device, BLOCK_SYS, BLOCK_USB, DEMOD_CTL, DEMOD_CTL_1, EEPROM_SIZE, GPD, GPO, GPOE, USB_EPA_CTL,
    USB_EPA_MAXPKT, USB_SYSCTL,
};
use super::error::{Result, RtlsdrError};
use super::error::RtlsdrError::RtlsdrErr;
use super::tuners::r82xx::{
    R82xx, R820T_TUNER_ID, R820T_TUNER_INFO, R828D_TUNER_ID, R828D_TUNER_INFO, R828D_XTAL_FREQ,
    R82XX_IF_FREQ,
};
use super::tuners::{i2c_read_reg, NoTuner, Tuner, TunerBus, TunerInfo};
use super::{DirectSampleMode, TunerGain};

const DEF_RTL_XTAL_FREQ: u32 = 28_800_000;
const MIN_RTL_XTAL_FREQ: u32 = DEF_RTL_XTAL_FREQ - 1000;
const MAX_RTL_XTAL_FREQ: u32 = DEF_RTL_XTAL_FREQ + 1000;

pub(crate) const FIR_LEN: usize = 16;
const DEFAULT_FIR: &[i32; FIR_LEN] = &[
    -54, -36, -41, -40, -32, -14, 14, 53, // i8
    101, 156, 215, 273, 327, 372, 404, 421, // i12
];

#[derive(Debug)]
pub struct RtlSdrCore {
    handle: Device,
    tuner: Box<dyn Tuner>,
    is_blog_v4: bool,
    freq: u32, // Hz
    rate: u32, // Hz
    bw: u32,
    direct_sampling: DirectSampleMode,
    xtal: u32,
    tuner_xtal: u32,
    offset_freq: u32,
    corr: i32, // PPM
    force_bt: bool,
    force_ds: bool,
    #[allow(dead_code)]
    fir: [i32; FIR_LEN],
}

impl RtlSdrCore {
    pub fn new(handle: Device) -> Self {
        Self {
            handle,
            tuner: Box::new(NoTuner {}),
            is_blog_v4: false,
            freq: 0,
            rate: 0,
            bw: 0,
            xtal: DEF_RTL_XTAL_FREQ,
            tuner_xtal: DEF_RTL_XTAL_FREQ,
            direct_sampling: DirectSampleMode::Off,
            offset_freq: 0,
            corr: 0,
            force_bt: false,
            force_ds: false,
            fir: *DEFAULT_FIR,
        }
    }

    pub fn init(&mut self) -> Result<()> {
        self.handle.test_write()?;
        step("Initialising the demodulator", self.init_baseband())?;
        step("Turning the I2C repeater on", self.set_i2c_repeater(true))?;

        let is_blog_v4 = self
            .handle
            .usb_strings()
            .map(|(manufact, product, _)| {
                matches!(manufact.as_deref(), Some("RTLSDRBlog"))
                    && matches!(product.as_deref(), Some("Blog V4"))
            })
            .unwrap_or(false);
        self.is_blog_v4 = is_blog_v4;

        let kind = match probe_tuner(&self.handle) {
            Some(kind) => {
                info!("Got tuner {:?}", kind);
                kind
            }
            None => {
                error!("No supported tuner found");
                let _ = self.set_i2c_repeater(false);
                return Err(RtlsdrError::UnknownTuner);
            }
        };

        let tuner: Box<dyn Tuner> = match kind {
            TunerKind::R820T => Box::new(R82xx::new_r820t()),
            TunerKind::R828D => Box::new(R82xx::new_r828d(is_blog_v4)),
        };

        self.tuner_xtal = match (kind, is_blog_v4) {
            (TunerKind::R828D, false) => R828D_XTAL_FREQ,
            _ => self.xtal,
        };

        self.tuner = tuner;
        self.tuner.set_xtal_freq(self.get_tuner_xtal_freq())?;

        step("Setting the demodulator up for the tuner", self.set_up_for_tuner(kind))?;

        let mut buf: [u8; EEPROM_SIZE] = [0; EEPROM_SIZE];
        let read = self.handle.read_eeprom(&mut buf, 0, EEPROM_SIZE);
        if let Err(e) = &read {
            warn!(
                "The EEPROM could not be read ({}): no bias-tee or direct-sampling setting from it",
                e
            );
        }
        (self.force_bt, self.force_ds) = eeprom_flags(read.ok().map(|_| &buf));

        info!("Init tuner");
        if let Err(e) = self.tuner.init(&self.handle) {
            warn!("The tuner's init failed ({}); going on, as librtlsdr does", e);
        }

        step("Turning the I2C repeater off", self.set_i2c_repeater(false))?;
        info!("Init complete");
        Ok(())
    }

    fn set_up_for_tuner(&mut self, kind: TunerKind) -> Result<()> {
        if kind.is_r82xx() {
            // disable Zero-IF mode
            self.handle.demod_write_reg(1, 0xb1, 0x1a, 1)?;
            // only enable In-phase ADC input
            self.handle.demod_write_reg(0, 0x08, 0x4d, 1)?;
            // the R82XX uses 3.57 MHz IF for 6 MHz mode
            self.set_if_freq(R82XX_IF_FREQ)?;
            // enable spectrum inversion
            self.handle.demod_write_reg(1, 0x15, 0x01, 1)?;
        }
        Ok(())
    }

    pub fn is_blog_v4(&self) -> bool {
        self.is_blog_v4
    }

    pub fn get_tuner_gains(&self) -> Result<Vec<i32>> {
        self.tuner.get_gains()
    }

    pub fn read_tuner_gain(&self) -> Result<i32> {
        self.set_i2c_repeater(true)?;
        let gain = self.tuner.read_gain(&self.handle)?;
        self.set_i2c_repeater(false)?;
        Ok(gain)
    }

    pub fn set_tuner_gain(&mut self, gain: TunerGain) -> Result<()> {
        self.set_i2c_repeater(true)?;
        let res = self.tuner.set_gain(&self.handle, gain);
        let rep = self.set_i2c_repeater(false);
        res?;
        rep
    }

    pub fn reset_buffer(&self) -> Result<()> {
        self.handle.write_reg(BLOCK_USB, USB_EPA_CTL, 0x1002, 2)?;
        self.handle.write_reg(BLOCK_USB, USB_EPA_CTL, 0x0000, 2)?;
        Ok(())
    }

    pub fn get_center_freq(&self) -> u32 {
        self.freq
    }

    pub fn set_center_freq(&mut self, freq: u32) -> Result<()> {
        if !matches!(self.direct_sampling, DirectSampleMode::Off) {
            self.set_if_freq(freq)?;
        } else {
            self.set_i2c_repeater(true)?;
            let res = self.tuner.set_freq(&self.handle, freq - self.offset_freq);
            let rep = self.set_i2c_repeater(false);
            res?;
            rep?;
        }
        self.freq = freq;
        Ok(())
    }

    pub fn set_if_freq(&self, freq: u32) -> Result<()> {
        let rtl_xtal: u32 = self.get_xtal_freq();
        let base = 1u32 << 22;
        let if_freq: i32 = -(freq as f64 * base as f64 / rtl_xtal as f64) as i32;

        let tmp = ((if_freq >> 16) as u16) & 0x3f;
        self.handle.demod_write_reg(1, 0x19, tmp, 1)?;
        let tmp = ((if_freq >> 8) as u16) & 0xff;
        self.handle.demod_write_reg(1, 0x1a, tmp, 1)?;
        let tmp = if_freq as u16 & 0xff;
        self.handle.demod_write_reg(1, 0x1b, tmp, 1)?;
        Ok(())
    }

    pub fn get_freq_correction(&self) -> i32 {
        self.corr
    }

    pub fn set_freq_correction(&mut self, ppm: i32) -> Result<()> {
        if self.corr == ppm {
            return Ok(());
        }
        self.corr = ppm;
        self.set_sample_freq_correction(ppm)?;
        self.tuner.set_xtal_freq(self.get_tuner_xtal_freq())?;
        if self.freq != 0 {
            self.set_center_freq(self.freq)?;
        }
        Ok(())
    }

    pub fn get_sample_rate(&self) -> u32 {
        self.rate
    }

    pub fn set_sample_rate(&mut self, rate: u32) -> Result<()> {
        if rate <= 225_000 || rate > 3_200_000 || (rate > 300_000 && rate <= 900_000) {
            return Err(RtlsdrErr(format!("Invalid sample rate: {} Hz", rate)));
        }

        let rsamp_ratio = (self.xtal as u128 * 2_u128.pow(22) / rate as u128) & 0x0ffffffc;
        info!(
            "set_sample_rate: rate: {}, xtal: {}, rsamp_ratio: {}",
            rate, self.xtal, rsamp_ratio
        );
        let real_resamp_ratio = rsamp_ratio | ((rsamp_ratio & 0x08000000) << 1);
        let real_rate = (self.xtal as u128 * 2_u128.pow(22)) as f64 / real_resamp_ratio as f64;
        if rate as f64 != real_rate {
            info!("Exact sample rate is {} Hz", real_rate);
        }
        self.rate = real_rate as u32;

        self.set_i2c_repeater(true)?;
        let val = if self.bw > 0 { self.bw } else { self.rate };
        let bw_res = self.tuner.set_bandwidth(&self.handle, val, self.rate);
        let rep_res = self.set_i2c_repeater(false);
        bw_res?;
        rep_res?;

        if Self::tuner_is_r82xx(self.tuner.get_info()?.id) {
            self.set_if_freq(self.tuner.get_if_freq()?)?;
            if self.freq != 0 {
                self.set_center_freq(self.freq)?;
            }
        }

        let mut tmp: u16 = (rsamp_ratio >> 16) as u16;
        self.handle.demod_write_reg(1, 0x9f, tmp, 2)?;
        tmp = (rsamp_ratio & 0xffff) as u16;
        self.handle.demod_write_reg(1, 0xa1, tmp, 2)?;

        self.set_sample_freq_correction(self.corr)?;

        // Reset demod (bit 3, soft_rst)
        self.handle.demod_write_reg(1, 0x01, 0x14, 1)?;
        self.handle.demod_write_reg(1, 0x01, 0x10, 1)?;

        Ok(())
    }

    pub fn set_tuner_bandwidth(&mut self, mut bw: u32) -> Result<()> {
        bw = if bw > 0 { bw } else { self.rate };
        self.set_i2c_repeater(true)?;
        let res = self.tuner.set_bandwidth(&self.handle, bw, self.rate);
        let rep = self.set_i2c_repeater(false);
        res?;
        rep?;
        if Self::tuner_is_r82xx(self.tuner.get_info()?.id) {
            self.set_if_freq(self.tuner.get_if_freq()?)?;
            if self.freq != 0 {
                self.set_center_freq(self.freq)?;
            }
        }
        self.bw = bw;
        Ok(())
    }

    pub fn set_bias_tee(&self, on: bool) -> Result<()> {
        self.set_gpio(0, on)
    }

    pub fn get_xtal_freq(&self) -> u32 {
        (self.xtal as f64 * (1.0 + self.corr as f64 / 1e6)).round() as u32
    }

    pub fn get_tuner_xtal_freq(&self) -> u32 {
        (self.tuner_xtal as f64 * (1.0 + self.corr as f64 / 1e6)).round() as u32
    }

    #[allow(dead_code)]
    pub fn set_xtal_freq(&mut self, rtl_freq: u32, tuner_freq: u32) -> Result<()> {
        if rtl_freq > 0 && !(MIN_RTL_XTAL_FREQ..=MAX_RTL_XTAL_FREQ).contains(&rtl_freq) {
            return Err(RtlsdrErr(format!(
                "set_xtal_freq error: rtl_freq {} out of bounds",
                rtl_freq
            )));
        }
        if rtl_freq > 0 && self.xtal != rtl_freq {
            self.xtal = rtl_freq;
            if self.rate != 0 {
                self.set_sample_rate(self.rate)?;
            }
        }

        if self.tuner.get_xtal_freq()? != tuner_freq {
            self.tuner_xtal = if tuner_freq == 0 { self.xtal } else { tuner_freq };
            self.tuner.set_xtal_freq(self.get_tuner_xtal_freq())?;
            if self.freq != 0 {
                self.set_center_freq(self.freq)?;
            }
        }
        Ok(())
    }

    fn init_baseband(&self) -> Result<()> {
        // Initialize USB
        self.handle.write_reg(BLOCK_USB, USB_SYSCTL, 0x09, 1)?;
        self.handle.write_reg(BLOCK_USB, USB_EPA_MAXPKT, 0x0002, 2)?;
        self.handle.write_reg(BLOCK_USB, USB_EPA_CTL, 0x1002, 2)?;

        // Power-on demod
        self.handle.write_reg(BLOCK_SYS, DEMOD_CTL_1, 0x22, 1)?;
        self.handle.write_reg(BLOCK_SYS, DEMOD_CTL, 0xe8, 1)?;

        // Reset demod (bit 3, soft_rst)
        self.handle.reset_demod()?;

        // Disable spectrum inversion and adjust channel rejection
        self.handle.demod_write_reg(1, 0x15, 0x00, 1)?;
        self.handle.demod_write_reg(1, 0x16, 0x00, 2)?;

        // Clear DDC shift and IF registers
        for i in 0..5 {
            self.handle.demod_write_reg(1, 0x16 + i, 0x00, 1)?;
        }
        self.set_fir(DEFAULT_FIR)?;

        // Enable SDR mode, disable DAGC (bit 5)
        self.handle.demod_write_reg(0, 0x19, 0x05, 1)?;

        // Init FSM state-holding register
        self.handle.demod_write_reg(1, 0x93, 0xf0, 1)?;
        self.handle.demod_write_reg(1, 0x94, 0x0f, 1)?;

        // Disable AGC (en_dagc, bit 0)
        self.handle.demod_write_reg(1, 0x11, 0x00, 1)?;

        // Disable RF and IF AGC loop
        self.handle.demod_write_reg(1, 0x04, 0x00, 1)?;

        // Disable PID filter
        self.handle.demod_write_reg(0, 0x61, 0x60, 1)?;

        // opt_adc_iq = 0, default ADC_I/ADC_Q datapath
        self.handle.demod_write_reg(0, 0x06, 0x80, 1)?;

        // Enable Zero-IF mode, DC cancellation, and IQ estimation/compensation
        self.handle.demod_write_reg(1, 0xb1, 0x1b, 1)?;

        // Disable 4.096 MHz clock output on pin TP_CK0
        self.handle.demod_write_reg(0, 0x0d, 0x83, 1)?;

        Ok(())
    }

    pub fn deinit_baseband(&mut self) -> Result<()> {
        if !self.force_bt {
            let _ = self.set_bias_tee(false);
        }
        // Deinitialize tuner
        self.set_i2c_repeater(true)?;
        let res = self.tuner.exit(&self.handle);
        let rep = self.set_i2c_repeater(false);
        res?;
        rep?;

        // Power-off demodulator and ADCs
        self.handle.write_reg(BLOCK_SYS, DEMOD_CTL, 0x20, 1)?;
        Ok(())
    }

    fn set_sample_freq_correction(&self, ppm: i32) -> Result<()> {
        let offs = (-ppm * 2_i32.pow(24) / 1_000_000) as i16;
        self.handle.demod_write_reg(1, 0x3f, (offs & 0xff) as u16, 1)?;
        self.handle.demod_write_reg(1, 0x3e, ((offs >> 8) & 0x3f) as u16, 1)?;
        Ok(())
    }

    fn set_gpio(&self, gpio_pin: u8, mut on: bool) -> Result<()> {
        if self.force_bt {
            on = true;
        }
        self.set_gpio_output(gpio_pin)?;
        self.set_gpio_bit(gpio_pin, on)
    }

    fn set_gpio_bit(&self, mut gpio: u8, val: bool) -> Result<()> {
        gpio = 1 << gpio;
        let mut r = self.handle.read_reg(BLOCK_SYS, GPO, 1)?;
        r = if val { r | gpio as u16 } else { r & !gpio as u16 };
        self.handle.write_reg(BLOCK_SYS, GPO, r, 1)?;
        Ok(())
    }

    fn set_gpio_output(&self, mut gpio: u8) -> Result<()> {
        gpio = 1 << gpio;
        let mut r = self.handle.read_reg(BLOCK_SYS, GPD, 1)?;
        self.handle.write_reg(BLOCK_SYS, GPD, r & !gpio as u16, 1)?;
        r = self.handle.read_reg(BLOCK_SYS, GPOE, 1)?;
        self.handle.write_reg(BLOCK_SYS, GPOE, r | gpio as u16, 1)?;
        Ok(())
    }

    fn set_i2c_repeater(&self, enable: bool) -> Result<()> {
        let val = if enable { 0x18 } else { 0x10 };
        self.handle.demod_write_reg(1, 0x01, val, 1).map(|_| ())
    }

    pub fn set_fir(&self, fir: &[i32; FIR_LEN]) -> Result<()> {
        const TMP_LEN: usize = 20;
        let mut tmp: [u8; TMP_LEN] = [0; TMP_LEN];
        for i in 0..8 {
            let val = fir[i];
            if !(-128..=127).contains(&val) {
                return Err(RtlsdrErr(format!("i8 FIR coefficient out of bounds: {}", val)));
            }
            tmp[i] = val as u8;
        }
        for i in (0..8).step_by(2) {
            let val0 = fir[8 + i];
            let val1 = fir[8 + i + 1];
            if !(-2048..=2047).contains(&val0) || !(-2048..=2047).contains(&val1) {
                return Err(RtlsdrErr(format!("i12 FIR coefficient out of bounds: {val0}, {val1}")));
            }
            tmp[8 + i * 3 / 2] = (val0 >> 4) as u8;
            tmp[8 + i * 3 / 2 + 1] = ((val0 << 4) | ((val1 >> 8) & 0x0f)) as u8;
            tmp[8 + i * 3 / 2 + 2] = val1 as u8;
        }

        for (i, &byte) in tmp.iter().enumerate() {
            self.handle.demod_write_reg(1, 0x1c + i as u16, byte as u16, 1)?;
        }
        Ok(())
    }

    fn tuner_is_r82xx(tuner_id: &str) -> bool {
        matches!(tuner_id, R820T_TUNER_ID | R828D_TUNER_ID)
    }

    pub fn get_tuner_id(&self) -> Result<&str> {
        Ok(self.tuner.get_info()?.id)
    }

    pub fn get_tuner_name(&self) -> Result<String> {
        let name = self.tuner.get_info()?.name;
        if self.is_blog_v4 {
            Ok(format!("{name} (Blog V4)"))
        } else {
            Ok(name.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TunerKind {
    R820T,
    R828D,
}

impl TunerKind {
    fn is_r82xx(self) -> bool {
        matches!(self, TunerKind::R820T | TunerKind::R828D)
    }
}

fn step<T>(what: &str, r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        error!("{} failed: {}", what, e);
    }
    r
}

pub(crate) fn eeprom_flags(eeprom: Option<&[u8; EEPROM_SIZE]>) -> (bool, bool) {
    match eeprom {
        Some(buf) => (buf[7] & 0x02 == 0, buf[7] & 0x01 != 0),
        None => (false, false),
    }
}

pub(crate) fn probe_tuner(bus: &dyn TunerBus) -> Option<TunerKind> {
    let found = |tuner: &TunerInfo, mask: u8| {
        info!(
            "Probing for the {} at I2C address {:#04x}, register {:#04x}",
            tuner.name, tuner.i2c_addr, tuner.check_addr
        );
        match i2c_read_reg(bus, tuner.i2c_addr, tuner.check_addr) {
            Some(val) => val & mask == tuner.check_val,
            None => {
                info!("No answer at I2C address {:#02x}", tuner.i2c_addr);
                false
            }
        }
    };
    if found(&R820T_TUNER_INFO, 0xff) {
        return Some(TunerKind::R820T);
    }
    if found(&R828D_TUNER_INFO, 0xff) {
        return Some(TunerKind::R828D);
    }
    None
}
