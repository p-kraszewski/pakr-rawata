#![allow(dead_code)]
#![allow(clippy::identity_op)]

use std::fmt;
use std::mem::MaybeUninit;
use std::{io, path::Path};

#[cfg(target_os = "freebsd")]
#[path = "freebsd.rs"]
mod os;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod os;

trait RawAta
where
    Self: std::marker::Sized,
{
    fn open<P: AsRef<Path>>(dev: P) -> io::Result<Self>;
    fn close(&mut self);
    fn raw_read(&mut self, sector: u64, buffer: &mut [u8]) -> io::Result<()>;
    fn raw_write(&mut self, sector: u64, buffer: &[u8]) -> io::Result<()>;
    fn raw_info(&mut self, ident: *mut IdentifyDeviceData) -> io::Result<()>;
}

#[derive(Copy, Clone)]
#[repr(C, packed)]
pub struct IdentifyDeviceData {
    general_configuration: u16,
    number_of_cylinders: u16,
    reserved1: u16,
    number_of_heads: u16,
    unformatted_bytes_per_track: u16,
    unformatted_bytes_per_sector: u16,
    sectors_per_track: u16,
    vendor_unique1: [u16; 3],
    serial_number: [u8; 20],
    buffer_type: u16,
    buffer_sector_size: u16,
    number_of_ecc_bytes: u16,
    firmware_revision: [u8; 8],
    model_number: [u8; 40],
    maximum_block_transfer: u8,
    vendor_unique2: u8,
    double_word_io: u16,
    capabilities: u16,
    reserved2: u16,
    vendor_unique3: u8,
    pio_cycle_timing_mode: u8,
    vendor_unique4: u8,
    dma_cycle_timing_mode: u8,
    translation_fields_valid: u16,
    number_of_current_cylinders: u16,
    number_of_current_heads: u16,
    current_sectors_per_track: u16,
    current_sector_capacity: u32,
    current_multi_sector_setting: u16,
    user_addressable_sectors: u32,
    single_word_dmasupport: u8,
    single_word_dmaactive: u8,
    multi_word_dmasupport: u8,
    multi_word_dmaactive: u8,
    advanced_piomodes: u8,
    reserved4: u8,
    minimum_mwxfer_cycle_time: u16,
    recommended_mwxfer_cycle_time: u16,
    minimum_piocycle_time: u16,
    minimum_piocycle_time_iordy: u16,
    reserved5: [u16; 2],
    release_time_overlapped: u16,
    release_time_service_command: u16,
    major_revision: u16,
    minor_revision: u16,
    max_queue_depth: u16,
    sata_capability: u16,
    reserved6: [u16; 9],
    command_support: u16,
    command_enable: u16,
    utral_dma_mode: u16,
    reserved7: [u16; 11],
    lba48bit: [u16; 4],
    reserved8: [u16; 23],
    special_functions_enabled: u16,
    reserved9: [u16; 128], // 128-255
}

impl IdentifyDeviceData {
    pub fn get_size(&self) -> u64 {
        u64::from(self.lba48bit[0]) << 0
            | u64::from(self.lba48bit[1]) << 16
            | u64::from(self.lba48bit[2]) << 32
            | u64::from(self.lba48bit[3]) << 48
    }

    pub fn get_model(&self) -> String {
        Self::swap_string(&self.model_number)
    }

    pub fn get_serial(&self) -> String {
        Self::swap_string(&self.serial_number)
    }

    pub fn get_firmware(&self) -> String {
        Self::swap_string(&self.firmware_revision)
    }

    #[inline]
    fn swap_bytes(buffer: &[u8]) -> Vec<u8> {
        assert_eq!(buffer.len() % 2, 0);

        let mut ans = Vec::with_capacity(buffer.len());

        for pair in buffer.chunks(2) {
            ans.push(pair[1]);
            ans.push(pair[0]);
        }
        ans
    }

    #[inline]
    fn swap_string(buffer: &[u8]) -> String {
        let swapped = Self::swap_bytes(buffer);
        String::from(String::from_utf8_lossy(swapped.as_slice()).trim())
    }
}

impl fmt::Debug for IdentifyDeviceData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = self.get_size();
        let model = self.get_model();
        let serial = self.get_serial();
        let firmware = self.get_firmware();

        f.debug_struct("IdentifyDeviceData")
            .field("sectors", &size)
            .field("model", &model)
            .field("firmware", &firmware)
            .field("serial", &serial)
            .finish()
    }
}

pub struct Device(os::ATA);

impl Device {
    #[inline]
    pub fn open<P>(dev: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        Ok(Device(os::ATA::open(dev)?))
    }

    #[inline]
    pub fn close(&mut self) {
        self.0.close();
    }

    #[inline]
    pub fn read(&mut self, sector: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.0.raw_read(sector, buffer)
    }

    #[inline]
    pub fn write(&mut self, sector: u64, buffer: &[u8]) -> io::Result<()> {
        self.0.raw_write(sector, buffer)
    }

    #[inline]
    pub fn info(&mut self) -> io::Result<IdentifyDeviceData> {
        let mut u_ident = MaybeUninit::<IdentifyDeviceData>::uninit();
        let ident = unsafe {
            self.0.raw_info(u_ident.as_mut_ptr())?;
            u_ident.assume_init()
        };

        Ok(ident)
    }
}

#[cfg(test)]
mod tests {
    use std::{mem, path};

    use super::*;

    #[cfg(target_os = "freebsd")]
    fn get_def_drive() -> &'static str {
        "/dev/ada0"
    }

    #[cfg(target_os = "linux")]
    fn get_def_drive() -> &'static str {
        "/dev/sda"
    }

    #[test]
    fn check_struct_sizes() {
        assert_eq!(
            mem::size_of::<IdentifyDeviceData>(),
            os::SECTOR_BYTES,
            "IdentifyDeviceData size not 512"
        );
    }

    #[test]
    fn check_drive_id() -> io::Result<()> {
        let dp = path::Path::new(get_def_drive());

        let mut dh = Device::open(dp)?;
        let id = dh.info()?;

        println!("{:?}", id);
        Ok(())
    }
}
