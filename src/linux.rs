//! Biblioteka umożliwiający "surowy" dostęp do dysku.
//!
//! * Odczyt dowolnego sektora bez sprawdzenia poprawnego zakresu (błąd jest
//! wtedy zgłaszany przez sam kontroler dysku) i z pominięciem wszystkich
//! cache'y systemu operacyjnego
//!
//! * Odczyt numeru seryjnego, modelu, oznaczenia firmware i raportowanej
//! pojemności dysku
//!
//! Operacje wykonywane są za pośrednictwem ioctl-i `SG_IO` (odczyt sektora) i
//! `HDIO_DRIVE_CMD` (odczyt metryki dysku)

#![allow(dead_code)]
#![allow(clippy::identity_op)]

use std::{ffi::CString, io, path::Path, ptr};

use libc::{self, c_int, c_ulong, ioctl};

use crate::RawAta;

pub const SECTOR_BYTES: usize = 512;
pub const MAX_TRANSFER_SECTORS: u64 = 65_536;
pub const MAX_TRANSFER_BYTES: usize = MAX_TRANSFER_SECTORS as usize * SECTOR_BYTES;

const HDIO_DRIVE_CMD: c_ulong = 0x031f;
const SG_IO: c_ulong = 0x2285;

const SG_ATA_16: u8 = 0x85;
const SG_ATA_16_LEN: u8 = 16;
const SG_ATA_LBA48: u8 = 1;
const SG_ATA_PROTO_DMA: u8 = 6 << 1;

const SG_FLAG_DIRECT_IO: u32 = 1;

const SG_CDB2_TLEN_NSECT: u8 = 2 << 0;
const SG_CDB2_TLEN_SECTORS: u8 = 1 << 2;
const SG_CDB2_TDIR_TO_DEV: u8 = 0 << 3;
const SG_CDB2_TDIR_FROM_DEV: u8 = 1 << 3;

const SG_DXFER_NONE: i32 = -1;
const SG_DXFER_TO_DEV: i32 = -2;
const SG_DXFER_FROM_DEV: i32 = -3;
const SG_DXFER_TO_FROM_DEV: i32 = -4;

pub(super) struct ATA(c_int);

#[repr(C, packed)]
struct Task {
    command: u8,
    sector: u8,
    feature: u8,
    nsector: u8,
    buffer: [u8; 512],
}

#[repr(C, packed)]
struct SgTaskHdr<BT> {
    interface_id: u32,
    dxfer_direction: i32,
    cmd_len: u8,
    mx_sb_len: u8,
    iovec_count: u16,
    dxfer_len: u32,
    dxferp: BT,
    cmdp: *mut u8,
    sbp: *mut u8,
    timeout: u32,
    flags: u32,
    pack_id: u32,
    usr_ptr: *mut u8,
    status: u8,
    masked_status: u8,
    msg_status: u8,
    sb_len_wr: u8,
    host_status: u16,
    driver_status: u16,
    resid: u32,
    duration: u32,
    info: u32,
}

impl RawAta for ATA {
    fn open<P>(dev: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        use std::os::unix::ffi::OsStrExt;
        let device = CString::new(dev.as_ref().as_os_str().as_bytes()).unwrap();

        let h = unsafe { libc::open(device.as_ptr(), libc::O_DIRECT | libc::O_RDONLY) };
        if h < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(ATA(h))
    }

    fn close(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }

    fn raw_read(&mut self, sector: u64, buffer: &mut [u8]) -> io::Result<()> {
        #![allow(unused_parens)]
        let mut cdb = [0u8; 16];
        let mut sb = [0u8; 32];

        // Wielokrotność sektora
        assert_eq!(buffer.len() % SECTOR_BYTES, 0);

        // Nie więcej niż maksymalny transfer
        assert!(buffer.len() <= MAX_TRANSFER_BYTES);

        let count = (buffer.len() / SECTOR_BYTES) as u32;

        // Nawet nie PYTAJCIE o kolejność bajtów w polu zawierającym numer
        // sektora (-_-,)

        cdb[0] = SG_ATA_16;
        cdb[1] = SG_ATA_LBA48 | SG_ATA_PROTO_DMA;
        cdb[2] = SG_CDB2_TLEN_NSECT | SG_CDB2_TLEN_SECTORS | SG_CDB2_TDIR_FROM_DEV;
        cdb[3] = 0; // FEAT_H
        cdb[4] = 0; // FEAT_L
        cdb[5] = (count >> 8) as u8; // NSect_H     = nsect08..16
        cdb[6] = (count >> 0) as u8; // NSect_L     = nsect00..07
        cdb[7] = (sector >> 24) as u8; // hob.lbal  = sector24..31
        cdb[8] = (sector >> 0) as u8; // lob.lbal   = sector00..07
        cdb[9] = (sector >> 32) as u8; // hob.lbam  = sector32..39
        cdb[10] = (sector >> 8) as u8; // lob.lbam  = sector08..15
        cdb[11] = (sector >> 40) as u8; // hob.lbah = sector40..47
        cdb[12] = (sector >> 16) as u8; // lob.lbah = sector16..23
        cdb[13] = 0b1110_0000; // LBA, DRV0
        cdb[14] = 0x25; // READ DMA EXT/READ SECT EXT

        let task = SgTaskHdr {
            interface_id: 'S' as u32,
            dxfer_direction: SG_DXFER_FROM_DEV,
            cmd_len: SG_ATA_16_LEN,
            mx_sb_len: sb.len() as u8,

            iovec_count: 0,
            dxfer_len: 512 * count,
            dxferp: buffer.as_mut_ptr(),
            cmdp: &mut cdb[0] as *mut u8,
            sbp: &mut sb[0] as *mut u8,
            timeout: 1000, // ms
            flags: SG_FLAG_DIRECT_IO,
            pack_id: sector as u32,
            usr_ptr: ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };

        let ans = unsafe { ioctl(self.0, SG_IO, &task) };

        if ans < 0 {
            return Err(io::Error::last_os_error());
        }

        if sb[0] != 0 {
            return Err(sg_error_to_io(sb[1]));
        }

        Ok(())
    }

    fn raw_write(&mut self, sector: u64, buffer: &[u8]) -> io::Result<()> {
        #![allow(unused_parens)]
        let mut cdb = [0u8; 16];
        let mut sb = [0u8; 32];

        // Wielokrotność sektora
        assert_eq!(buffer.len() % SECTOR_BYTES, 0);

        // Nie więcej niż maksymalny transfer
        assert!(buffer.len() <= MAX_TRANSFER_BYTES);

        let count = (buffer.len() / SECTOR_BYTES) as u32;

        // Nawet nie PYTAJCIE o kolejność bajtów w polu zawierającym numer
        // sektora (-_-,)

        cdb[0] = SG_ATA_16;
        cdb[1] = SG_ATA_LBA48 | SG_ATA_PROTO_DMA;
        cdb[2] = SG_CDB2_TLEN_NSECT | SG_CDB2_TLEN_SECTORS | SG_CDB2_TDIR_TO_DEV;
        cdb[3] = 0; // FEAT_H
        cdb[4] = 0; // FEAT_L
        cdb[5] = (count >> 8) as u8; // NSect_H     = nsect08..16
        cdb[6] = (count >> 0) as u8; // NSect_L     = nsect00..07
        cdb[7] = (sector >> 24) as u8; // hob.lbal  = sector24..31
        cdb[8] = (sector >> 0) as u8; // lob.lbal   = sector00..07
        cdb[9] = (sector >> 32) as u8; // hob.lbam  = sector32..39
        cdb[10] = (sector >> 8) as u8; // lob.lbam  = sector08..15
        cdb[11] = (sector >> 40) as u8; // hob.lbah = sector40..47
        cdb[12] = (sector >> 16) as u8; // lob.lbah = sector16..23
        cdb[13] = 0b1110_0000; // LBA, DRV0
        cdb[14] = 0x35; // WRITE DMA EXT/READ SECT EXT

        let task = SgTaskHdr {
            interface_id: 'S' as u32,
            dxfer_direction: SG_DXFER_TO_DEV,
            cmd_len: SG_ATA_16_LEN,
            mx_sb_len: sb.len() as u8,

            iovec_count: 0,
            dxfer_len: 512 * count,
            dxferp: buffer.as_ptr(),
            cmdp: &mut cdb[0] as *mut u8,
            sbp: &mut sb[0] as *mut u8,
            timeout: 1000, // ms
            flags: SG_FLAG_DIRECT_IO,
            pack_id: sector as u32,
            usr_ptr: ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };

        let ans = unsafe { ioctl(self.0, SG_IO, &task) };

        if ans < 0 {
            return Err(io::Error::last_os_error());
        }

        if sb[0] != 0 {
            return Err(sg_error_to_io(sb[1]));
        }

        Ok(())
    }

    fn raw_info(&mut self, ident: *mut super::IdentifyDeviceData) -> io::Result<()> {
        let t = Task {
            command: 0xEC,
            sector: 0x00,
            feature: 0x00,
            nsector: 0x01,
            buffer: [0; 512],
        };
        let ans = unsafe { ioctl(self.0, HDIO_DRIVE_CMD, &t) };

        if ans < 0 {
            return Err(io::Error::last_os_error());
        }

        unsafe {
            std::ptr::copy(
                t.buffer.as_ptr() as *const super::IdentifyDeviceData,
                ident,
                1,
            );
        }
        Ok(())
    }
}

fn sg_error_to_io(err: u8) -> io::Error {
    assert!(err <= 15);
    io::Error::new(
        io::ErrorKind::Other,
        match err {
            0 => "NO_SENSE",
            1 => "RECOVERED_ERROR",
            2 => "NOT_READY",
            3 => "MEDIUM_ERROR",
            4 => "HARDWARE_ERROR",
            5 => "ILLEGAL_REQUEST",
            6 => "UNIT_ATTENTION",
            7 => "DATA_PROTECT",
            8 => "BLANK_CHECK",
            9 => "VENDOR_SPECIFIC",
            10 => "COPY_ABORTED",
            11 => "ABORTED_COMMAND",
            12 => "OTHER",
            13 => "VOLUME_OVERFLOW",
            14 => "MISCOMPARE",
            15 => "COMPLETE",
            _ => unimplemented!("Shouldn't be here"),
        },
    )
}

impl Drop for ATA {
    /// Zamknięcie uchwytu do napędu
    fn drop(&mut self) {
        self.close();
    }
}
