//! Builds the firmware tables that uACPI expects to find, around the AML tables from the input.
//!
//! uACPI starts from an RSDP, follows it to an XSDT, and from there finds a FADT (which points at
//! the DSDT and the FACS) and any SSDTs. The input file only has AML tables, so the rest is made
//! up here. The structure is the same one uACPI's own test runner uses.
//!
//! "Physical" addresses in these tables are just the addresses of host memory; see
//! [`super::host`], which maps them back to themselves.

use crate::tables::Table;
use uacpi_sys as sys;

const SDT_HEADER_LEN: usize = 36;
const SDT_CHECKSUM_OFFSET: usize = 9;

/// The tables uACPI is given, kept alive (and at fixed addresses) for as long as this exists.
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

        // uACPI checks table checksums, and hand-made or truncated input often gets them wrong.
        // The `acpi` crate ignores checksums, so they are corrected here rather than reported.
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

    /// The address to report to uACPI as the RSDP.
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

/// The bytes of a plain-old-data C struct.
fn struct_bytes<T: Copy>(value: &T) -> Vec<u8> {
    // SAFETY: `T` is one of the `#[repr(C, packed)]` structs from the bindings, which have no
    // padding and no invalid bit patterns, and the slice covers exactly the struct.
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) }.to_vec()
}

/// Set the checksum byte at `offset` so that all of `bytes` sums to zero.
fn set_checksum(bytes: &mut [u8], offset: usize) {
    bytes[offset] = 0;
    let sum = bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
    bytes[offset] = 0u8.wrapping_sub(sum);
}

fn chars<const N: usize>(bytes: &[u8; N]) -> [sys::uacpi_char; N] {
    bytes.map(|b| b as sys::uacpi_char)
}

const OEM_TABLE_ID: &[u8; 8] = b"ACPITEST";

fn header(signature: &[u8; 4], length: usize, revision: u8, oem_id: [u8; 6]) -> sys::acpi_sdt_hdr {
    sys::acpi_sdt_hdr {
        signature: chars(signature),
        length: length as u32,
        revision,
        oemid: chars(&oem_id),
        oem_table_id: chars(OEM_TABLE_ID),
        ..Default::default()
    }
}

/// A DSDT with no AML in it.
fn empty_dsdt() -> Vec<u8> {
    let mut bytes = vec![0u8; SDT_HEADER_LEN];
    bytes[0..4].copy_from_slice(b"DSDT");
    bytes[4..8].copy_from_slice(&(SDT_HEADER_LEN as u32).to_le_bytes());
    bytes[8] = 2;
    bytes[10..16].copy_from_slice(b"ACPITS");
    bytes[16..24].copy_from_slice(OEM_TABLE_ID);
    bytes
}

fn facs() -> Vec<u8> {
    let facs = sys::acpi_facs {
        signature: chars(b"FACS"),
        length: size_of::<sys::acpi_facs>() as u32,
        ..Default::default()
    };
    struct_bytes(&facs)
}

fn fadt(dsdt: u64, facs: u64, revision: u8, oem_id: [u8; 6]) -> Vec<u8> {
    // The register blocks are given made-up addresses; port I/O goes nowhere (see `host`).
    let fadt = sys::acpi_fadt {
        hdr: header(
            b"FACP",
            size_of::<sys::acpi_fadt>(),
            revision.max(6),
            oem_id,
        ),
        pm1a_evt_blk: 0xDEAD,
        pm1_evt_len: 4,
        pm1a_cnt_blk: 0xFFEE,
        pm1_cnt_len: 2,
        pm2_cnt_blk: 0xCCDD,
        pm2_cnt_len: 1,
        gpe0_blk: 0xDEAD,
        gpe0_blk_len: 0x20,
        gpe1_blk: 0xBEEF,
        gpe1_blk_len: 0x20,
        gpe1_base: 128,
        x_dsdt: dsdt,
        x_firmware_ctrl: facs,
        ..Default::default()
    };
    let mut bytes = struct_bytes(&fadt);
    set_checksum(&mut bytes, SDT_CHECKSUM_OFFSET);
    bytes
}

fn xsdt(entries: &[u64], revision: u8, oem_id: [u8; 6]) -> Vec<u8> {
    let length = SDT_HEADER_LEN + size_of_val(entries);
    let mut bytes = struct_bytes(&header(b"XSDT", length, revision, oem_id));
    for entry in entries {
        bytes.extend_from_slice(&entry.to_le_bytes());
    }
    set_checksum(&mut bytes, SDT_CHECKSUM_OFFSET);
    bytes
}

fn rsdp(xsdt: u64, oem_id: [u8; 6]) -> Vec<u8> {
    let rsdp = sys::acpi_rsdp {
        signature: chars(b"RSD PTR "),
        oemid: chars(&oem_id),
        revision: 2,
        length: size_of::<sys::acpi_rsdp>() as u32,
        xsdt_addr: xsdt,
        ..Default::default()
    };
    let mut bytes = struct_bytes(&rsdp);
    // The first checksum covers the original 20-byte structure, the second the whole thing.
    let first_checksum_offset = 8;
    let first_len = 20;
    let extended_offset = 32;
    bytes[first_checksum_offset] = 0;
    let sum = bytes[..first_len]
        .iter()
        .fold(0u8, |s, b| s.wrapping_add(*b));
    bytes[first_checksum_offset] = 0u8.wrapping_sub(sum);
    set_checksum(&mut bytes, extended_offset);
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
        let rsdp = at(fw.rsdp_address(), 36);
        assert_eq!(&rsdp[..8], b"RSD PTR ");
        assert_eq!(sum(&rsdp[..20]), 0);
        assert_eq!(sum(rsdp), 0);

        let xsdt_address = u64_at(rsdp, 24);
        let xsdt_length =
            u32::from_le_bytes(at(xsdt_address, 8)[4..8].try_into().unwrap()) as usize;
        let xsdt = at(xsdt_address, xsdt_length);
        assert_eq!(&xsdt[..4], b"XSDT");
        assert_eq!(sum(xsdt), 0);
        let entries: Vec<u64> = xsdt[36..]
            .chunks(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();

        let fadt_address = entries[0];
        let fadt = at(fadt_address, size_of::<sys::acpi_fadt>());
        assert_eq!(&fadt[..4], b"FACP");
        assert_eq!(sum(fadt), 0);
        let dsdt_offset = std::mem::offset_of!(sys::acpi_fadt, x_dsdt);
        (u64_at(fadt, dsdt_offset), entries[1..].to_vec())
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
                .any(|&(start, len)| start == rsdp && len == 36)
        );
        // RSDP, XSDT, FADT, FACS and the DSDT.
        assert_eq!(ranges.len(), 5);
    }
}
