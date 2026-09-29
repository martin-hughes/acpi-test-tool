DefinitionBlock ("", "DSDT", 2, "ACPI", "TESTTABL", 0xF0F0F0F0) {
    Method (MAIN, 0, NotSerialized) {
        Return (0)
    }

    Method (NORT) {
        MAIN()
    }

    Method (RET3) {
        Return (3)
    }
}
