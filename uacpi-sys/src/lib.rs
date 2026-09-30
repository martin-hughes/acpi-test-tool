//! Raw FFI bindings to [uACPI](https://github.com/uACPI/uACPI).
//!
//! The C library is built from the `ext-libs/uACPI` submodule by `build.rs`. The host must provide
//! the `uacpi_kernel_*` functions declared in `uacpi/kernel_api.h`.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::all, unsafe_op_in_unsafe_fn, missing_docs)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
