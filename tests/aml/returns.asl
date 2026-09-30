// Methods that return each kind of value, and one that takes an argument.
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
}
