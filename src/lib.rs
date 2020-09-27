//! Raw access to hard disks for Linux and FreeBSD. For technical information refer
//! to [ATA/ATAPI Command Set](http://t13.org/Documents/UploadedDocuments/docs2017/di529r18-ATAATAPI_Command_Set_-_4.pdf)
//! guide.
//!
//! # Warning
//!
//! **it bypasses all OS security checks and all software caches. You can kill the data on
//! your HDD in a blink of an eye. The _only_ protection is that it requires administrative
//! privilege to run.**
//!
//! # Supported operations
//!
//! - read sectors using `READ_DMA_EXT` (ATA cmd 0x25, documentation chapter 7.21),
//! - write sectors using `WRITE_DMA_EXT` (ATA cmd 0x35, documentation chapter 7.57)
//! - identify drive using `IDENTIFY_DEVICE` (ATA cmd 0xEC, documentation chapter 7.13, including a
//!   detailed description of returned structure).
//!
//! On Linux uses `SG` subsystem, on FreeBSD uses `CAM` subsystem.
//!
//! # Note
//!
//! *In theory*, a single ATA DMA transfer is limited to 65536 sectors (32MiB for 512B sectors).
//! Sector count is 16 bit and a full 65536 sector transfer is indicated by a sector count of
//! 0x0000).
//!
//! *In practice* operating system enforces much lower limit, in the range of a few hundred
//! kilobytes.
//!
//! On FreeBSD I managed to achieve stable transfers of 8MB at a time by re-compiling
//! kernel with custom configuration:
//! ```text
//! include GENERIC
//!
//! ident           BIGDMA
//!
//! options         DFLTPHYS=(16U*1024*1024)
//! options         MAXPHYS=(32U*1024*1024)
//! ```
//!
//! On Linux I didn't find any accessible tunable to bump-up the maximal DMA transfer size,
//! neither compile-time nor run-time.
//!
//! # TODO
//! - support sector sizes different than 512 bytes
//!

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

/// ATA standard IDENTIFY_DEVICE structure.
///
/// It is described in the table 55 of [ATA/ATAPI Command Set](http://t13.org/Documents/UploadedDocuments/docs2017/di529r18-ATAATAPI_Command_Set_-_4.pdf).
///
/// Due to a 16-bit bus architecture of ATA, that structure contains 256 16-bit words, not 512
/// bytes. Side effect of this layout is that all strings have pairwise swapped letters. String
/// "Abcdef" is stored in memory as "bAdcfe",
///
/// Numeric values are stored as LE-LE, that is bytes within word are little-endian and for
/// multi-word values words themselves are also little-endian. This is demonstrated in
/// [`IdentifyDeviceData::get_sector_count`].
#[derive(Copy, Clone)]
pub struct IdentifyDeviceData([u16; 256]);

impl IdentifyDeviceData {
    /// Return total sector count of disk
    pub fn get_sector_count(&self) -> u64 {
        let ptr = self.0[100..=103].as_ptr() as *const u64;

        // Always safe, source is always 4*u16 long
        let len = unsafe { *ptr };

        // Convert for non-LE hosts, no-op for LE-hosts
        u64::from_le(len)
    }

    /// Return model info of disk
    pub fn get_model(&self) -> String {
        Self::swap_string(&self.0[27..=46])
    }

    /// Return serial number of disk
    pub fn get_serial(&self) -> String {
        Self::swap_string(&self.0[10..=19])
    }

    /// Return firmware revision of disk
    pub fn get_firmware(&self) -> String {
        Self::swap_string(&self.0[23..=26])
    }

    /// Read range fixing byte order (bytes are always pairwise swapped, regardless of host being
    /// LE or BE)
    #[inline]
    fn swap_bytes(buffer: &[u16]) -> Vec<u8> {
        let mut ans = Vec::with_capacity(buffer.len() * 2);

        for word in buffer {
            ans.push(((word >> 8) & 0xFF) as u8);
            ans.push((word & 0xFF) as u8);
        }
        ans
    }

    /// Convert un-swapped range into string assuming it is utf8-ish.
    #[inline]
    fn swap_string(buffer: &[u16]) -> String {
        let swapped = Self::swap_bytes(buffer);
        String::from(String::from_utf8_lossy(swapped.as_slice()).trim())
    }
}

impl fmt::Debug for IdentifyDeviceData {
    /// Return basic drive information
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = self.get_sector_count();
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

/// Attached ATA device
pub struct Device(os::ATA);

impl Device {
    /// Open device pointed by a specific path.
    ///
    /// **DO NOT** use _partition_ references here (like `/dev/sda1` on Linux or `/dev/ada0p1` on
    /// FreeBSD). Use **only** _raw disk_ references, like  `/dev/sda` on Linux or `/dev/ada0` on
    /// FreeBSD.
    #[inline]
    pub fn open<P>(dev: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        Ok(Device(os::ATA::open(dev)?))
    }

    /// Close opened device
    #[inline]
    pub fn close(&mut self) {
        self.0.close();
    }

    /// Read sector(s) from disk.
    ///
    /// Buffer size **must** be multiple of sector size. **It bypasses all protections and
    /// caches/buffers.**
    #[inline]
    pub fn read(&mut self, sector: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.0.raw_read(sector, buffer)
    }

    /// Write sector(s) to disk.
    ///
    /// Buffer size **must** be multiple of sector size. **It bypasses all protections and
    /// caches/buffers.**
    #[inline]
    pub fn write(&mut self, sector: u64, buffer: &[u8]) -> io::Result<()> {
        self.0.raw_write(sector, buffer)
    }

    /// Get identification record from disk.
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
