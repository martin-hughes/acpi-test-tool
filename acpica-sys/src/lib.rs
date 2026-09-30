//! Raw FFI bindings to [ACPICA](https://github.com/acpica/acpica).
//!
//! The C library is built from the `ext-libs/acpica` submodule by `build.rs`. The host must
//! provide the `AcpiOs*` functions declared in `acpiosxf.h`, and `acpica_host_log`, which
//! receives every message ACPICA prints (see `shim.c`).

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::all, unsafe_op_in_unsafe_fn, missing_docs, dead_code)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
