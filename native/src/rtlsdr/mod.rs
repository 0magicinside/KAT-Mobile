// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Embedded MPL-2.0 RTL-SDR control driver (RTL2832U + Rafael Micro R820T/R820T2/R828D,
//! including RTL-SDR Blog V3/V4), adapted from desktop KAT's `rtl-sdr-rs` to operate over
//! Android's USB Host API (`UsbDeviceConnection.controlTransfer`) without libusb.

pub mod device;
pub mod error;
mod rtlsdr;
pub mod tuners;

pub use device::{is_known_device, Device, UsbTransport, KNOWN_DEVICES};
pub use error::{Result, RtlsdrError};
use rtlsdr::RtlSdrCore;
pub use tuners::r82xx::{R820T_TUNER_ID, R828D_TUNER_ID};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunerGain {
    Auto,
    Manual(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSampleMode {
    Off,
    On,
    OnSwap,
}

#[derive(Debug)]
pub struct RtlSdr {
    sdr: RtlSdrCore,
}

impl RtlSdr {
    pub fn open(transport: Box<dyn UsbTransport>) -> Result<Self> {
        let dev = Device::new(transport);
        let mut sdr = RtlSdrCore::new(dev);
        sdr.init()?;
        Ok(Self { sdr })
    }

    pub fn close(&mut self) -> Result<()> {
        self.sdr.deinit_baseband()
    }

    pub fn reset_buffer(&self) -> Result<()> {
        self.sdr.reset_buffer()
    }

    pub fn get_center_freq(&self) -> u32 {
        self.sdr.get_center_freq()
    }

    pub fn set_center_freq(&mut self, freq: u32) -> Result<()> {
        self.sdr.set_center_freq(freq)
    }

    pub fn get_tuner_gains(&self) -> Result<Vec<i32>> {
        self.sdr.get_tuner_gains()
    }

    pub fn set_tuner_gain(&mut self, gain: TunerGain) -> Result<()> {
        self.sdr.set_tuner_gain(gain)
    }

    pub fn get_freq_correction(&self) -> i32 {
        self.sdr.get_freq_correction()
    }

    pub fn set_freq_correction(&mut self, ppm: i32) -> Result<()> {
        self.sdr.set_freq_correction(ppm)
    }

    pub fn get_sample_rate(&self) -> u32 {
        self.sdr.get_sample_rate()
    }

    pub fn set_sample_rate(&mut self, rate: u32) -> Result<()> {
        self.sdr.set_sample_rate(rate)
    }

    pub fn set_tuner_bandwidth(&mut self, bw: u32) -> Result<()> {
        self.sdr.set_tuner_bandwidth(bw)
    }

    pub fn set_bias_tee(&self, on: bool) -> Result<()> {
        self.sdr.set_bias_tee(on)
    }

    pub fn get_tuner_id(&self) -> Result<&str> {
        self.sdr.get_tuner_id()
    }

    pub fn get_tuner_name(&self) -> Result<String> {
        self.sdr.get_tuner_name()
    }

    pub fn is_blog_v4(&self) -> bool {
        self.sdr.is_blog_v4()
    }
}
