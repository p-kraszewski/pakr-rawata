//! Moduł zapewniający niskopoziomowy dostęp do dysku bezpośrednio przez
//! komendy ATA

use std::{
    ffi::CString,
    io::{self, Error, ErrorKind},
    mem,
    os::raw::c_char,
    path::{self, Path},
    ptr,
};

use crate::RawAta;

mod camlib {
    #![allow(clippy::unreadable_literal)]
    #![allow(clippy::const_static_lifetime)]
    #![allow(clippy::useless_transmute)]
    #![allow(clippy::cast_lossless)]
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]

    use std::marker::{Send, Sync};

    include!(concat!(env!("OUT_DIR"), "/libcam-bind.rs"));
    unsafe impl Send for ccb {}

    unsafe impl Sync for ccb {}

    unsafe impl Send for cam_device {}

    unsafe impl Sync for cam_device {}
}

pub const SECTOR_BYTES: usize = 512;
pub const MAX_TRANSFER_SECTORS: u64 = 65_536;
pub const MAX_TRANSFER_BYTES: usize = MAX_TRANSFER_SECTORS as usize * SECTOR_BYTES;

pub(super) struct ATA {
    cam: *mut camlib::cam_device,
    ccb: *mut camlib::ccb,
}

impl ATA {
    #[inline]
    fn ccb_clear_all_except_hdr(&mut self) {
        const CCB_S: usize = mem::size_of::<camlib::ccb>();
        const CCB_H_S: usize = mem::size_of::<camlib::ccb_hdr>();

        debug_assert!(CCB_H_S < CCB_S);
        unsafe {
            let ccb = self.ccb as *mut u8;
            ptr::write_bytes(ccb.add(CCB_H_S), 0u8, CCB_S - CCB_H_S);
        }
    }
}

impl RawAta for ATA {
    fn open<P>(dev: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        use std::os::unix::ffi::OsStrExt;
        let cdev = CString::new(dev.as_ref().as_os_str().as_bytes())?;
        let mut name: [c_char; 30] = [0; 30];
        let mut unit = 0;

        let rc = unsafe { camlib::cam_get_device(cdev.as_ptr(), name.as_mut_ptr(), 30, &mut unit) };
        if rc == -1 {
            return Err(Error::last_os_error());
        }

        let cam = unsafe {
            camlib::cam_open_spec_device(
                name.as_ptr(),
                unit,
                camlib::O_RDWR as i32,
                ptr::null_mut(),
            )
        };
        if cam.is_null() {
            return Err(Error::last_os_error());
        }

        let ccb = unsafe { camlib::cam_getccb(cam) };
        if ccb.is_null() {
            return Err(Error::last_os_error());
        }

        Ok(ATA { cam, ccb })
    }

    fn close(&mut self) {
        if !self.ccb.is_null() {
            unsafe {
                camlib::cam_freeccb(self.ccb);
            }
        }

        if !self.cam.is_null() {
            unsafe {
                camlib::cam_close_spec_device(self.cam);
            }
        }
    }

    fn raw_read(&mut self, sector: u64, buffer: &mut [u8]) -> io::Result<()> {
        #![allow(unused_parens)]

        let len = buffer.len();

        debug_assert!(len >= SECTOR_BYTES && len <= MAX_TRANSFER_BYTES);
        debug_assert!(len % SECTOR_BYTES == 0);

        self.ccb_clear_all_except_hdr();

        unsafe {
            (*self.ccb).ataio.cmd.command = camlib::ATA_READ_DMA48 as u8;
            (*self.ccb).ataio.cmd.flags = (camlib::CAM_ATAIO_NEEDRESULT
                | camlib::CAM_ATAIO_DMA
                | camlib::CAM_ATAIO_48BIT) as u8;
            (*self.ccb).ataio.cmd.sector_count = (len / 512) as u8;
            (*self.ccb).ataio.cmd.sector_count_exp = ((len / 512) >> 8) as u8;
            (*self.ccb).ataio.cmd.lba_low = (sector) as u8;
            (*self.ccb).ataio.cmd.lba_mid = (sector >> 8) as u8;
            (*self.ccb).ataio.cmd.lba_high = (sector >> 16) as u8;
            (*self.ccb).ataio.cmd.lba_low_exp = (sector >> 24) as u8;
            (*self.ccb).ataio.cmd.lba_mid_exp = (sector >> 32) as u8;
            (*self.ccb).ataio.cmd.lba_high_exp = (sector >> 40) as u8;
            (*self.ccb).ataio.cmd.device = camlib::ATA_DEV_LBA as u8;
            (*self.ccb).ataio.cmd.control = 0;
            (*self.ccb).ataio.cmd.features_exp = 0;
            (*self.ccb).ataio.cmd.features = 0;

            (*self.ccb).ataio.ccb_h.func_code = camlib::xpt_opcode_XPT_ATA_IO;
            (*self.ccb).ataio.ccb_h.flags =
                camlib::ccb_flags_CAM_DIR_IN | camlib::ccb_flags_CAM_DEV_QFRZDIS;
            (*self.ccb).ataio.ccb_h.retry_count = 1;
            (*self.ccb).ataio.ccb_h.cbfcnp = None;
            (*self.ccb).ataio.ccb_h.timeout = 5000;

            (*self.ccb).ataio.data_ptr = buffer.as_mut_ptr();
            (*self.ccb).ataio.dxfer_len = len as u32;
            (*self.ccb).ataio.ata_flags = 0;
        }
        let rc = unsafe { camlib::cam_send_ccb(self.cam, self.ccb) };
        if rc < 0 {
            return Err(Error::last_os_error());
        }

        if unsafe { (*self.ccb).ataio.res.status & 0x01 != 0 } {
            return Err(Error::new(ErrorKind::InvalidData, "CCB execute failed"));
        }

        Ok(())
    }

    fn raw_write(&mut self, sector: u64, buffer: &[u8]) -> io::Result<()> {
        #![allow(unused_parens)]

        let len = buffer.len();

        debug_assert!(len >= SECTOR_BYTES && len <= MAX_TRANSFER_BYTES);
        debug_assert!(len % SECTOR_BYTES == 0);

        self.ccb_clear_all_except_hdr();

        unsafe {
            (*self.ccb).ataio.cmd.command = camlib::ATA_WRITE_DMA48 as u8;
            (*self.ccb).ataio.cmd.flags = (camlib::CAM_ATAIO_NEEDRESULT
                | camlib::CAM_ATAIO_DMA
                | camlib::CAM_ATAIO_48BIT) as u8;
            (*self.ccb).ataio.cmd.sector_count = (len / 512) as u8;
            (*self.ccb).ataio.cmd.sector_count_exp = ((len / 512) >> 8) as u8;
            (*self.ccb).ataio.cmd.lba_low = (sector) as u8;
            (*self.ccb).ataio.cmd.lba_mid = (sector >> 8) as u8;
            (*self.ccb).ataio.cmd.lba_high = (sector >> 16) as u8;
            (*self.ccb).ataio.cmd.lba_low_exp = (sector >> 24) as u8;
            (*self.ccb).ataio.cmd.lba_mid_exp = (sector >> 32) as u8;
            (*self.ccb).ataio.cmd.lba_high_exp = (sector >> 40) as u8;
            (*self.ccb).ataio.cmd.device = camlib::ATA_DEV_LBA as u8;
            (*self.ccb).ataio.cmd.control = 0;
            (*self.ccb).ataio.cmd.features_exp = 0;
            (*self.ccb).ataio.cmd.features = 0;

            (*self.ccb).ataio.ccb_h.func_code = camlib::xpt_opcode_XPT_ATA_IO;
            (*self.ccb).ataio.ccb_h.flags =
                camlib::ccb_flags_CAM_DIR_OUT | camlib::ccb_flags_CAM_DEV_QFRZDIS;
            (*self.ccb).ataio.ccb_h.retry_count = 1;
            (*self.ccb).ataio.ccb_h.cbfcnp = None;
            (*self.ccb).ataio.ccb_h.timeout = 5000;

            (*self.ccb).ataio.data_ptr = buffer.as_ptr() as *mut u8;
            (*self.ccb).ataio.dxfer_len = len as u32;
            (*self.ccb).ataio.ata_flags = 0;
        }
        let rc = unsafe { camlib::cam_send_ccb(self.cam, self.ccb) };
        if rc < 0 {
            return Err(Error::last_os_error());
        }

        if unsafe { (*self.ccb).ataio.res.status & 0x01 != 0 } {
            return Err(Error::new(ErrorKind::InvalidData, "CCB execute failed"));
        }

        Ok(())
    }

    fn raw_info(&mut self, ident: *mut super::IdentifyDeviceData) -> io::Result<()> {
        #![allow(unused_parens)]

        self.ccb_clear_all_except_hdr();

        unsafe {
            (*self.ccb).ataio.cmd.command = camlib::ATA_ATA_IDENTIFY as u8;
            (*self.ccb).ataio.cmd.flags =
                (camlib::CAM_ATAIO_NEEDRESULT | camlib::CAM_ATAIO_DMA) as u8;
            (*self.ccb).ataio.cmd.sector_count = 1;
            (*self.ccb).ataio.cmd.sector_count_exp = 0;
            (*self.ccb).ataio.cmd.device = camlib::ATA_DEV_LBA as u8;
            (*self.ccb).ataio.cmd.control = 0;
            (*self.ccb).ataio.cmd.features_exp = 0;
            (*self.ccb).ataio.cmd.features = 0;

            (*self.ccb).ataio.ccb_h.func_code = camlib::xpt_opcode_XPT_ATA_IO;
            (*self.ccb).ataio.ccb_h.flags =
                camlib::ccb_flags_CAM_DIR_IN | camlib::ccb_flags_CAM_DEV_QFRZDIS;
            (*self.ccb).ataio.ccb_h.retry_count = 1;
            (*self.ccb).ataio.ccb_h.cbfcnp = None;
            (*self.ccb).ataio.ccb_h.timeout = 5000;

            (*self.ccb).ataio.data_ptr = ident as *mut super::IdentifyDeviceData as *mut u8;
            (*self.ccb).ataio.dxfer_len = 512;
            (*self.ccb).ataio.ata_flags = 0;
        }
        let rc = unsafe { camlib::cam_send_ccb(self.cam, self.ccb) };
        if rc < 0 {
            return Err(Error::last_os_error());
        }

        if unsafe { (*self.ccb).ataio.res.status & 0x01 != 0 } {
            return Err(Error::new(ErrorKind::InvalidData, "CCB execute failed"));
        }

        Ok(())
    }
}

impl Drop for ATA {
    fn drop(&mut self) {
        self.close();
    }
}
