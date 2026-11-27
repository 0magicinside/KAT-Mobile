// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

pub mod r82xx;

use super::device::{Device, BLOCK_SYS, GPD, GPO, GPOE};
use super::error::Result;
use super::TunerGain;

#[derive(Debug, Clone, Copy)]
pub struct TunerInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub i2c_addr: u8,
    pub check_addr: u8,
    pub check_val: u8,
}

pub trait Tuner: Send + std::fmt::Debug {
    fn init(&mut self, handle: &Device) -> Result<()>;
    fn get_info(&self) -> Result<TunerInfo>;
    fn get_gains(&self) -> Result<Vec<i32>>;
    fn read_gain(&self, handle: &Device) -> Result<i32>;
    fn set_gain(&mut self, handle: &Device, gain: TunerGain) -> Result<()>;
    fn set_freq(&mut self, handle: &Device, freq: u32) -> Result<()>;
    fn set_bandwidth(&mut self, handle: &Device, bw: u32, rate: u32) -> Result<()>;
    fn get_if_freq(&self) -> Result<u32>;
    fn get_xtal_freq(&self) -> Result<u32>;
    fn set_xtal_freq(&mut self, freq: u32) -> Result<()>;
    fn exit(&mut self, handle: &Device) -> Result<()>;
}

/// What a tuner driver needs of the RTL2832U: its I2C bus (through the repeater) and GPIO pins.
pub trait TunerBus {
    fn write(&self, addr: u8, data: &[u8]) -> Result<usize>;
    fn read(&self, addr: u8, buf: &mut [u8]) -> Result<usize>;
    #[allow(dead_code)]
    fn set_gpio_bit(&self, gpio: u8, on: bool) -> Result<()>;
    #[allow(dead_code)]
    fn set_gpio_output(&self, gpio: u8) -> Result<()>;
}

impl TunerBus for Device {
    fn write(&self, addr: u8, data: &[u8]) -> Result<usize> {
        self.i2c_write(addr.into(), data)
    }

    fn read(&self, addr: u8, buf: &mut [u8]) -> Result<usize> {
        let len = buf.len() as u8;
        self.i2c_read(addr.into(), buf, len)
    }

    fn set_gpio_bit(&self, gpio: u8, on: bool) -> Result<()> {
        let bit = 1u16 << gpio;
        let r = self.read_reg(BLOCK_SYS, GPO, 1)?;
        self.write_reg(BLOCK_SYS, GPO, if on { r | bit } else { r & !bit }, 1)?;
        Ok(())
    }

    fn set_gpio_output(&self, gpio: u8) -> Result<()> {
        let bit = 1u16 << gpio;
        let r = self.read_reg(BLOCK_SYS, GPD, 1)?;
        self.write_reg(BLOCK_SYS, GPD, r & !bit, 1)?;
        let r = self.read_reg(BLOCK_SYS, GPOE, 1)?;
        self.write_reg(BLOCK_SYS, GPOE, r | bit, 1)?;
        Ok(())
    }
}

/// Read one register of the I2C device at `addr`; None when the device does not answer.
pub(crate) fn i2c_read_reg(bus: &dyn TunerBus, addr: u8, reg: u8) -> Option<u8> {
    let mut data = [0u8];
    bus.write(addr, &[reg]).ok()?;
    bus.read(addr, &mut data).ok()?;
    Some(data[0])
}

#[derive(Debug)]
pub struct NoTuner {}

impl Tuner for NoTuner {
    fn init(&mut self, _handle: &Device) -> Result<()> {
        Ok(())
    }
    fn get_info(&self) -> Result<TunerInfo> {
        Ok(TunerInfo {
            id: "",
            name: "",
            i2c_addr: 0,
            check_addr: 0,
            check_val: 0,
        })
    }
    fn get_gains(&self) -> Result<Vec<i32>> {
        Ok(vec![])
    }
    fn read_gain(&self, _handle: &Device) -> Result<i32> {
        Ok(0)
    }
    fn set_gain(&mut self, _handle: &Device, _gain: TunerGain) -> Result<()> {
        Ok(())
    }
    fn set_freq(&mut self, _handle: &Device, _freq: u32) -> Result<()> {
        Ok(())
    }
    fn set_bandwidth(&mut self, _handle: &Device, _bw: u32, _rate: u32) -> Result<()> {
        Ok(())
    }
    fn get_xtal_freq(&self) -> Result<u32> {
        Ok(0)
    }
    fn set_xtal_freq(&mut self, _freq: u32) -> Result<()> {
        Ok(())
    }
    fn get_if_freq(&self) -> Result<u32> {
        Ok(0)
    }
    fn exit(&mut self, _handle: &Device) -> Result<()> {
        Ok(())
    }
}
