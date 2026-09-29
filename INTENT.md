# Plan

## Outline

To create a tool similar to `acpiexec` that loads an AML file using an AML interpreter, then presents a TUI for the
user, allowing them to interrogate the ACPI Namespace or to execute methods.

## MVP:

A tool that:

* Takes one command line argument (the name of an AML binary file)
* Loads it using the `acpi` crate
* Prints out any errors
* Allows the user to enter commands:
  * `dump` will dump a tree view of the namespace
  * `find [name]` will search the namespace for `[name]`, and dump the object found, and its type.
  * `exit` to quit.

### The TUI

* The TUI will be REPL-style, with output printed and then the opportunity for a user to enter commands.
* `rustyline` or a similar crate should be used.

### Design decisions

* Input files are ACPI tables that include the 36-byte SDT header. A single file may contain several tables back to
  back. Only `DSDT`/`SSDT` tables are loaded; others are skipped with a message.
* The namespace is left as the loader produced it: no `_INI`, `_STA` etc. are run.
* `find` accepts either an absolute path (`\_SB_.PCI0`), which must match exactly, or a bare name / partial path
  (`PCI0`, `DEV0.FOO_`), which matches anywhere in the namespace. All matches are printed. Names shorter than four
  characters are padded with `_`.
* Output is a type and value summary (integers, strings, buffers, method arg counts, region and field details).
* `dump` prints an ASCII tree with the type of each node.
* Interpreters sit behind the small `AmlBackend` trait, which only loads tables and lists namespace entries. Searching
  and rendering are backend-neutral. `execute`/`set` would be added to the trait later.
* Host hardware access (memory, I/O, PCI) is stubbed with `NullHandler`, which returns zero.

### Tech stack

Use Rust and Cargo throughout. The `acpi` crate needs the nightly toolchain (`rust-toolchain.toml` selects it).

### Dependencies

The only restrictions on dependencies are:

* They must all be available on `crates.io` with no significant CVEs.
  * There are two exceptions, both from the `main` branch of `https://github.com/rust-osdev/acpi`:
    * the `acpi` crate itself, instead of the crates.io version.
    * `aml-test-tools`, which provides `NullHandler` and `new_interpreter`. It is not published to crates.io.

## Future work

> This is not needed now, but should be considered when thinking about architecture or other decisions
 
* Add command line arguments to choose which AML interpreter to use:
  * The `acpi` crate as in the MVP
  * `uACPI`
  * `ACPICA`
* an `execute [name]` command to allow running an AML method
* Some sort of "set" command allowing values in the namespace to be altered before running a method.
* Other commands to manipulate the namespace