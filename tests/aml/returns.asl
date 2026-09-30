// Methods that return each kind of value, and some that take arguments.
DefinitionBlock ("", "DSDT", 2, "ACPI", "RETURNS", 0x00000001) {
    Method (STR1, 0, NotSerialized) {
        Return ("hello")
    }

    Method (BUF1, 0, NotSerialized) {
        Return (Buffer () { 1, 2, 3 })
    }

    Method (PKG1, 0, NotSerialized) {
        Return (Package () { 1, "two", Buffer () { 4 } })
    }

    Method (TAK1, 1, NotSerialized) {
        Return (Arg0)
    }

    Method (ADD2, 2, NotSerialized) {
        Return (Arg0 + Arg1)
    }

    Method (CAT2, 2, NotSerialized) {
        Return (Concatenate (Arg0, Arg1))
    }
}
