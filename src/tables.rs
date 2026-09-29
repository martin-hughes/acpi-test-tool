//! Splits a binary image into ACPI tables. Independent of any AML interpreter.

const HEADER_LEN: usize = 36;

/// One ACPI table: the full bytes (header included) plus its signature.
pub struct Table {
    pub signature: String,
    pub bytes: Vec<u8>,
}

impl Table {
    /// The table body, without the 36-byte SDT header.
    pub fn body(&self) -> &[u8] {
        &self.bytes[HEADER_LEN..]
    }

    /// Whether the table contains AML (DSDT or SSDT).
    pub fn is_aml(&self) -> bool {
        self.signature == "DSDT" || self.signature == "SSDT"
    }
}

/// Split `bytes` into consecutive tables, each starting with an SDT header.
pub fn split_tables(bytes: &[u8]) -> Result<Vec<Table>, String> {
    let mut tables = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let rest = &bytes[offset..];
        if rest.len() < HEADER_LEN {
            return Err(format!(
                "{} trailing bytes are shorter than a table header",
                rest.len()
            ));
        }
        let length = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        if length < HEADER_LEN {
            return Err(format!(
                "table at offset {offset} has invalid length {length}"
            ));
        }
        if length > rest.len() {
            return Err(format!(
                "table at offset {offset} claims {length} bytes but only {} remain",
                rest.len()
            ));
        }
        let signature = String::from_utf8_lossy(&rest[0..4]).into_owned();
        tables.push(Table {
            signature,
            bytes: rest[..length].to_vec(),
        });
        offset += length;
    }
    Ok(tables)
}

/// Build a table from a signature and body. The checksum is not computed.
#[cfg(test)]
pub fn make_table(signature: &str, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; HEADER_LEN];
    out[0..4].copy_from_slice(signature.as_bytes());
    out[4..8].copy_from_slice(&((HEADER_LEN + body.len()) as u32).to_le_bytes());
    out[8] = 2;
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_multiple_tables() {
        let mut image = make_table("DSDT", &[1, 2, 3]);
        image.extend(make_table("SSDT", &[4]));
        let tables = split_tables(&image).unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].signature, "DSDT");
        assert_eq!(tables[0].body(), &[1, 2, 3]);
        assert!(tables[1].is_aml());
        assert_eq!(tables[1].body(), &[4]);
    }

    #[test]
    fn rejects_short_and_truncated() {
        assert!(split_tables(&[0; 10]).is_err());
        let mut image = make_table("DSDT", &[1, 2, 3]);
        image.pop();
        assert!(split_tables(&image).is_err());
    }

    #[test]
    fn rejects_bad_length() {
        let mut image = make_table("DSDT", &[]);
        image[4..8].copy_from_slice(&4u32.to_le_bytes());
        assert!(split_tables(&image).is_err());
    }
}
