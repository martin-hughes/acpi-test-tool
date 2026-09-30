//! Builds the firmware tables that the C interpreters (uACPI, ACPICA) expect to find, around the
//! AML tables from the input.
//!
//! Both start from an RSDP, follow it to an XSDT, and from there find a FADT (which points at the
//! DSDT and the FACS) and any SSDTs. The input file only has AML tables, so the rest is made up
//! here. The structure is the one used by uACPI's test runner and ACPICA's `acpiexec`.
//!
//! "Physical" addresses in these tables are just the addresses of host memory, so a backend's
//! host layer has to map them back to themselves (see [`Firmware::address_ranges`]).
//!
//! The layouts are written out here, rather than taken from either library's headers, so that
//! this works with whichever interpreters are compiled in. The tests check them against the
//! libraries' own structures.

// Only the C interpreters use this, and they are optional.
#![cfg_attr(not(any(feature = "uacpi", feature = "acpica")), allow(dead_code))]

use crate::tables::Table;

const SDT_HEADER_LEN: usize = 36;
const SDT_CHECKSUM_OFFSET: usize = 9;

/// Size of an ACPI 6.x FADT.
const FADT_LEN: usize = 276;
/// Size of an ACPI 2.0+ RSDP.
const RSDP_LEN: usize = 36;
/// Size of a FACS.
const FACS_LEN: usize = 64;

const OEM_TABLE_ID: &[u8; 8] = b"ACPITEST";

/// The tables handed to an interpreter, kept alive (and at fixed addresses) for as long as this
/// exists.
pub struct Firmware {
    /// Each table is its own heap allocation, so its address doesn't change if this struct moves.
    buffers: Vec<Box<[u8]>>,
    rsdp: u64,
}

impl Firmware {
    /// Wrap `tables` (DSDTs and SSDTs) in the tables needed to reach them. Anything unusual is
    /// added to `problems`.
    pub fn build(tables: &[Table], problems: &mut Vec<String>) -> Firmware {
        let mut buffers: Vec<Box<[u8]>> = Vec::new();

        // The first DSDT is the DSDT. There can only be one, so extra ones are dropped.
        let mut dsdt: Option<Vec<u8>> = None;
        let mut ssdts: Vec<Vec<u8>> = Vec::new();
        for table in tables {
            match table.signature.as_str() {
                "DSDT" if dsdt.is_none() => dsdt = Some(table.bytes.clone()),
                "DSDT" => problems.push("ignoring extra DSDT; there can only be one".into()),
                _ => ssdts.push(table.bytes.clone()),
            }
        }
        // ACPI requires a DSDT, so a file with only SSDTs gets an empty one.
        let dsdt = dsdt.unwrap_or_else(empty_dsdt);
        let (revision, oem_id) = (dsdt[8], <[u8; 6]>::try_from(&dsdt[10..16]).unwrap());

        let mut add = |mut bytes: Vec<u8>, fix: bool| -> u64 {
            if fix {
                set_checksum(&mut bytes, SDT_CHECKSUM_OFFSET);
            }
            let buffer = bytes.into_boxed_slice();
            let address = buffer.as_ptr() as u64;
            buffers.push(buffer);
            address
        };

        // The interpreters check table checksums, and hand-made or truncated input often gets
        // them wrong. The `acpi` crate ignores checksums, so they are corrected here rather than
        // reported.
        let dsdt_address = add(dsdt, true);
        let ssdt_addresses: Vec<u64> = ssdts.into_iter().map(|ssdt| add(ssdt, true)).collect();

        let facs_address = add(facs(), false);
        let fadt_address = add(fadt(dsdt_address, facs_address, revision, oem_id), false);

        let mut entries = vec![fadt_address];
        entries.extend(&ssdt_addresses);
        let xsdt_address = add(xsdt(&entries, revision, oem_id), false);

        let rsdp = add(rsdp(xsdt_address, oem_id), false);
        Firmware { buffers, rsdp }
    }

    /// The address to report to the interpreter as the RSDP.
    pub fn rsdp_address(&self) -> u64 {
        self.rsdp
    }

    /// The (start, length) of every table, for the host's memory mapper.
    pub fn address_ranges(&self) -> Vec<(usize, usize)> {
        self.buffers
            .iter()
            .map(|b| (b.as_ptr() as usize, b.len()))
            .collect()
    }
}

/// Set the checksum byte at `offset` so that all of `bytes` sums to zero.
fn set_checksum(bytes: &mut [u8], offset: usize) {
    bytes[offset] = 0;
    let sum = bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
    bytes[offset] = 0u8.wrapping_sub(sum);
}

/// Offsets of the fields that are set in an ACPI 6.x FADT.
mod fadt_field {
    pub const PM1A_EVT_BLK: usize = 56;
    pub const PM1A_CNT_BLK: usize = 64;
    pub const PM2_CNT_BLK: usize = 72;
    pub const GPE0_BLK: usize = 80;
    pub const GPE1_BLK: usize = 84;
    pub const PM1_EVT_LEN: usize = 88;
    pub const PM1_CNT_LEN: usize = 89;
    pub const PM2_CNT_LEN: usize = 90;
    pub const GPE0_BLK_LEN: usize = 92;
    pub const GPE1_BLK_LEN: usize = 93;
    pub const GPE1_BASE: usize = 94;
    pub const X_FIRMWARE_CTRL: usize = 132;
    pub const X_DSDT: usize = 140;
}

/// Offsets of the fields that are set in an RSDP.
mod rsdp_field {
    pub const CHECKSUM: usize = 8;
    pub const OEM_ID: usize = 9;
    pub const REVISION: usize = 15;
    pub const LENGTH: usize = 20;
    pub const XSDT_ADDRESS: usize = 24;
    pub const EXTENDED_CHECKSUM: usize = 32;
    /// The first checksum covers the original ACPI 1.0 structure, which was this long.
    pub const ORIGINAL_LEN: usize = 20;
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

/// An SDT header, with the length, revision and OEM fields filled in.
fn header(signature: &[u8; 4], length: usize, revision: u8, oem_id: [u8; 6]) -> Vec<u8> {
    let mut bytes = vec![0u8; SDT_HEADER_LEN];
    put(&mut bytes, 0, signature);
    put(&mut bytes, 4, &(length as u32).to_le_bytes());
    bytes[8] = revision;
    put(&mut bytes, 10, &oem_id);
    put(&mut bytes, 16, OEM_TABLE_ID);
    bytes
}

/// A DSDT with no AML in it.
fn empty_dsdt() -> Vec<u8> {
    header(b"DSDT", SDT_HEADER_LEN, 2, *b"ACPITS")
}

fn facs() -> Vec<u8> {
    let mut bytes = vec![0u8; FACS_LEN];
    put(&mut bytes, 0, b"FACS");
    put(&mut bytes, 4, &(FACS_LEN as u32).to_le_bytes());
    bytes
}

fn fadt(dsdt: u64, facs: u64, revision: u8, oem_id: [u8; 6]) -> Vec<u8> {
    use fadt_field::*;
    let mut bytes = header(b"FACP", FADT_LEN, revision.max(6), oem_id);
    bytes.resize(FADT_LEN, 0);

    // The register blocks are given made-up addresses; port I/O goes nowhere useful (see each
    // backend's host layer).
    put(&mut bytes, PM1A_EVT_BLK, &0xDEADu32.to_le_bytes());
    bytes[PM1_EVT_LEN] = 4;
    put(&mut bytes, PM1A_CNT_BLK, &0xFFEEu32.to_le_bytes());
    bytes[PM1_CNT_LEN] = 2;
    put(&mut bytes, PM2_CNT_BLK, &0xCCDDu32.to_le_bytes());
    bytes[PM2_CNT_LEN] = 1;
    put(&mut bytes, GPE0_BLK, &0xDEADu32.to_le_bytes());
    bytes[GPE0_BLK_LEN] = 0x20;
    put(&mut bytes, GPE1_BLK, &0xBEEFu32.to_le_bytes());
    bytes[GPE1_BLK_LEN] = 0x20;
    bytes[GPE1_BASE] = 128;
    put(&mut bytes, X_DSDT, &dsdt.to_le_bytes());
    put(&mut bytes, X_FIRMWARE_CTRL, &facs.to_le_bytes());

    set_checksum(&mut bytes, SDT_CHECKSUM_OFFSET);
    bytes
}

fn xsdt(entries: &[u64], revision: u8, oem_id: [u8; 6]) -> Vec<u8> {
    let length = SDT_HEADER_LEN + size_of_val(entries);
    let mut bytes = header(b"XSDT", length, revision, oem_id);
    for entry in entries {
        bytes.extend_from_slice(&entry.to_le_bytes());
    }
    set_checksum(&mut bytes, SDT_CHECKSUM_OFFSET);
    bytes
}

fn rsdp(xsdt: u64, oem_id: [u8; 6]) -> Vec<u8> {
    use rsdp_field::*;
    let mut bytes = vec![0u8; RSDP_LEN];
    put(&mut bytes, 0, b"RSD PTR ");
    put(&mut bytes, OEM_ID, &oem_id);
    bytes[REVISION] = 2;
    put(&mut bytes, LENGTH, &(RSDP_LEN as u32).to_le_bytes());
    put(&mut bytes, XSDT_ADDRESS, &xsdt.to_le_bytes());

    // The first checksum covers only the original 20-byte structure, the second the whole thing.
    set_checksum(&mut bytes[..ORIGINAL_LEN], CHECKSUM);
    set_checksum(&mut bytes, EXTENDED_CHECKSUM);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::make_table;

    fn table(signature: &str, body: &[u8]) -> Table {
        Table {
            signature: signature.into(),
            bytes: make_table(signature, body),
        }
    }

    /// The bytes at a "physical" address produced by `Firmware`.
    fn at(address: u64, len: usize) -> &'static [u8] {
        // SAFETY: the tests only pass addresses of buffers owned by a `Firmware` that outlives
        // the returned slice's use.
        unsafe { std::slice::from_raw_parts(address as *const u8, len) }
    }

    fn sum(bytes: &[u8]) -> u8 {
        bytes.iter().fold(0, |s, b| s.wrapping_add(*b))
    }

    fn u64_at(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    /// Walk RSDP -> XSDT and return (dsdt address, xsdt entry addresses).
    fn walk(fw: &Firmware) -> (u64, Vec<u64>) {
        let rsdp = at(fw.rsdp_address(), RSDP_LEN);
        assert_eq!(&rsdp[..8], b"RSD PTR ");
        assert_eq!(sum(&rsdp[..rsdp_field::ORIGINAL_LEN]), 0);
        assert_eq!(sum(rsdp), 0);

        let xsdt_address = u64_at(rsdp, rsdp_field::XSDT_ADDRESS);
        let xsdt_length =
            u32::from_le_bytes(at(xsdt_address, 8)[4..8].try_into().unwrap()) as usize;
        let xsdt = at(xsdt_address, xsdt_length);
        assert_eq!(&xsdt[..4], b"XSDT");
        assert_eq!(sum(xsdt), 0);
        let entries: Vec<u64> = xsdt[SDT_HEADER_LEN..]
            .chunks(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();

        let fadt_address = entries[0];
        let fadt = at(fadt_address, FADT_LEN);
        assert_eq!(&fadt[..4], b"FACP");
        assert_eq!(sum(fadt), 0);
        (u64_at(fadt, fadt_field::X_DSDT), entries[1..].to_vec())
    }

    fn signature(address: u64) -> String {
        String::from_utf8_lossy(&at(address, 4)[..4]).into_owned()
    }

    #[test]
    fn dsdt_and_ssdts_are_reachable_from_the_rsdp() {
        let tables = [
            table("DSDT", &[1, 2, 3]),
            table("SSDT", &[4]),
            table("SSDT", &[5, 6]),
        ];
        let mut problems = Vec::new();
        let fw = Firmware::build(&tables, &mut problems);
        assert!(problems.is_empty());

        let (dsdt, ssdts) = walk(&fw);
        assert_eq!(signature(dsdt), "DSDT");
        assert_eq!(&at(dsdt, 39)[36..], &[1, 2, 3]);
        assert_eq!(ssdts.len(), 2);
        assert!(ssdts.iter().all(|&s| signature(s) == "SSDT"));
        assert_eq!(&at(ssdts[1], 38)[36..], &[5, 6]);
    }

    #[test]
    fn the_fadt_points_at_the_facs() {
        let fw = Firmware::build(&[table("DSDT", &[1])], &mut Vec::new());
        let rsdp = at(fw.rsdp_address(), RSDP_LEN);
        let xsdt = at(u64_at(rsdp, rsdp_field::XSDT_ADDRESS), SDT_HEADER_LEN + 8);
        let fadt = at(u64_at(xsdt, SDT_HEADER_LEN), FADT_LEN);
        let facs = at(u64_at(fadt, fadt_field::X_FIRMWARE_CTRL), FACS_LEN);
        assert_eq!(&facs[..4], b"FACS");
        assert_eq!(
            u32::from_le_bytes(facs[4..8].try_into().unwrap()) as usize,
            FACS_LEN
        );
    }

    #[test]
    fn table_checksums_are_corrected() {
        // `make_table` doesn't compute a checksum, so this one is wrong to begin with.
        let tables = [table("DSDT", &[1, 2, 3])];
        assert_ne!(sum(&tables[0].bytes), 0);
        let fw = Firmware::build(&tables, &mut Vec::new());
        let (dsdt, _) = walk(&fw);
        assert_eq!(sum(at(dsdt, 39)), 0);
        // The caller's copy is untouched.
        assert_ne!(sum(&tables[0].bytes), 0);
    }

    #[test]
    fn a_missing_dsdt_is_replaced_by_an_empty_one() {
        let fw = Firmware::build(&[table("SSDT", &[7])], &mut Vec::new());
        let (dsdt, ssdts) = walk(&fw);
        assert_eq!(signature(dsdt), "DSDT");
        assert_eq!(at(dsdt, 36).len(), 36);
        assert_eq!(ssdts.len(), 1);
    }

    #[test]
    fn a_second_dsdt_is_reported_and_ignored() {
        let tables = [table("DSDT", &[1]), table("DSDT", &[2])];
        let mut problems = Vec::new();
        let fw = Firmware::build(&tables, &mut problems);
        assert_eq!(problems.len(), 1);
        let (dsdt, ssdts) = walk(&fw);
        assert_eq!(&at(dsdt, 37)[36..], &[1]);
        assert!(ssdts.is_empty());
    }

    #[test]
    fn address_ranges_cover_every_table() {
        let fw = Firmware::build(&[table("DSDT", &[1])], &mut Vec::new());
        let ranges = fw.address_ranges();
        let rsdp = fw.rsdp_address() as usize;
        assert!(
            ranges
                .iter()
                .any(|&(start, len)| start == rsdp && len == RSDP_LEN)
        );
        // RSDP, XSDT, FADT, FACS and the DSDT.
        assert_eq!(ranges.len(), 5);
    }

    /// The layouts above must agree with uACPI's own definitions.
    #[cfg(feature = "uacpi")]
    #[test]
    fn layouts_match_uacpi() {
        use std::mem::{offset_of, size_of};
        use uacpi_sys as sys;

        assert_eq!(size_of::<sys::acpi_fadt>(), FADT_LEN);
        assert_eq!(size_of::<sys::acpi_rsdp>(), RSDP_LEN);
        assert_eq!(size_of::<sys::acpi_facs>(), FACS_LEN);
        assert_eq!(size_of::<sys::acpi_sdt_hdr>(), SDT_HEADER_LEN);

        assert_eq!(offset_of!(sys::acpi_sdt_hdr, checksum), SDT_CHECKSUM_OFFSET);
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm1a_evt_blk),
            fadt_field::PM1A_EVT_BLK
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm1a_cnt_blk),
            fadt_field::PM1A_CNT_BLK
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm2_cnt_blk),
            fadt_field::PM2_CNT_BLK
        );
        assert_eq!(offset_of!(sys::acpi_fadt, gpe0_blk), fadt_field::GPE0_BLK);
        assert_eq!(offset_of!(sys::acpi_fadt, gpe1_blk), fadt_field::GPE1_BLK);
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm1_evt_len),
            fadt_field::PM1_EVT_LEN
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm1_cnt_len),
            fadt_field::PM1_CNT_LEN
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, pm2_cnt_len),
            fadt_field::PM2_CNT_LEN
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, gpe0_blk_len),
            fadt_field::GPE0_BLK_LEN
        );
        assert_eq!(
            offset_of!(sys::acpi_fadt, gpe1_blk_len),
            fadt_field::GPE1_BLK_LEN
        );
        assert_eq!(offset_of!(sys::acpi_fadt, gpe1_base), fadt_field::GPE1_BASE);
        assert_eq!(
            offset_of!(sys::acpi_fadt, x_firmware_ctrl),
            fadt_field::X_FIRMWARE_CTRL
        );
        assert_eq!(offset_of!(sys::acpi_fadt, x_dsdt), fadt_field::X_DSDT);

        assert_eq!(offset_of!(sys::acpi_rsdp, checksum), rsdp_field::CHECKSUM);
        assert_eq!(offset_of!(sys::acpi_rsdp, oemid), rsdp_field::OEM_ID);
        assert_eq!(offset_of!(sys::acpi_rsdp, revision), rsdp_field::REVISION);
        assert_eq!(offset_of!(sys::acpi_rsdp, length), rsdp_field::LENGTH);
        assert_eq!(
            offset_of!(sys::acpi_rsdp, xsdt_addr),
            rsdp_field::XSDT_ADDRESS
        );
        assert_eq!(
            offset_of!(sys::acpi_rsdp, extended_checksum),
            rsdp_field::EXTENDED_CHECKSUM
        );
    }

    /// ...and with ACPICA's.
    #[cfg(feature = "acpica")]
    #[test]
    fn layouts_match_acpica() {
        use acpica_sys as sys;
        use std::mem::{offset_of, size_of};

        assert_eq!(size_of::<sys::ACPI_TABLE_FADT>(), FADT_LEN);
        assert_eq!(size_of::<sys::ACPI_TABLE_RSDP>(), RSDP_LEN);
        assert_eq!(size_of::<sys::ACPI_TABLE_FACS>(), FACS_LEN);
        assert_eq!(size_of::<sys::ACPI_TABLE_HEADER>(), SDT_HEADER_LEN);

        assert_eq!(
            offset_of!(sys::ACPI_TABLE_HEADER, Checksum),
            SDT_CHECKSUM_OFFSET
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm1aEventBlock),
            fadt_field::PM1A_EVT_BLK
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm1aControlBlock),
            fadt_field::PM1A_CNT_BLK
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm2ControlBlock),
            fadt_field::PM2_CNT_BLK
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Gpe0Block),
            fadt_field::GPE0_BLK
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Gpe1Block),
            fadt_field::GPE1_BLK
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm1EventLength),
            fadt_field::PM1_EVT_LEN
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm1ControlLength),
            fadt_field::PM1_CNT_LEN
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Pm2ControlLength),
            fadt_field::PM2_CNT_LEN
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Gpe0BlockLength),
            fadt_field::GPE0_BLK_LEN
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Gpe1BlockLength),
            fadt_field::GPE1_BLK_LEN
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, Gpe1Base),
            fadt_field::GPE1_BASE
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_FADT, XFacs),
            fadt_field::X_FIRMWARE_CTRL
        );
        assert_eq!(offset_of!(sys::ACPI_TABLE_FADT, XDsdt), fadt_field::X_DSDT);

        assert_eq!(
            offset_of!(sys::ACPI_TABLE_RSDP, Checksum),
            rsdp_field::CHECKSUM
        );
        assert_eq!(offset_of!(sys::ACPI_TABLE_RSDP, OemId), rsdp_field::OEM_ID);
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_RSDP, Revision),
            rsdp_field::REVISION
        );
        assert_eq!(offset_of!(sys::ACPI_TABLE_RSDP, Length), rsdp_field::LENGTH);
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_RSDP, XsdtPhysicalAddress),
            rsdp_field::XSDT_ADDRESS
        );
        assert_eq!(
            offset_of!(sys::ACPI_TABLE_RSDP, ExtendedChecksum),
            rsdp_field::EXTENDED_CHECKSUM
        );
    }
}
