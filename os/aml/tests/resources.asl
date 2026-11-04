// Resource templates as firmware writes them, for the parser (src/resource.rs): an embedded controller's ports, a
// PCI root bridge's and old devices' resources, a touchpad on I2C with its GPIO interrupt.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "RES", 1)
{
    Name (EC00, ResourceTemplate ()
    {
        IO (Decode16, 0x0062, 0x0062, 0x00, 0x01)
        IO (Decode16, 0x0066, 0x0066, 0x00, 0x01)
    })
    Name (MIX0, ResourceTemplate ()
    {
        FixedIO (0x0060, 0x01)
        IRQNoFlags () {1}
        IRQ (Level, ActiveLow, Shared) {9, 11}
        Memory32Fixed (ReadWrite, 0xFED00000, 0x00000400)
        Memory32 (ReadOnly, 0x000C0000, 0x000DFFFF, 0x00000001, 0x00020000)
        Interrupt (ResourceConsumer, Edge, ActiveHigh, Exclusive, ,, ) {0x10, 0x11}
        DWordMemory (ResourceProducer, PosDecode, MinFixed, MaxFixed, Cacheable, ReadWrite,
            0x00000000, 0xC0000000, 0xFEBFFFFF, 0x00000000, 0x3EC00000)
        QWordMemory (ResourceProducer, PosDecode, MinFixed, MaxFixed, Cacheable, ReadWrite,
            0x0000000000000000, 0x0000000800000000, 0x0000000FFFFFFFFF, 0x0000000000000000, 0x0000000800000000)
        WordBusNumber (ResourceProducer, MinFixed, MaxFixed, PosDecode,
            0x0000, 0x0000, 0x00FF, 0x0000, 0x0100)
        WordIO (ResourceProducer, MinFixed, MaxFixed, PosDecode, EntireRange,
            0x0000, 0x0D00, 0xFFFF, 0x0000, 0xF300)
    })
    Name (TPD0, ResourceTemplate ()
    {
        I2cSerialBusV2 (0x002C, ControllerInitiated, 400000, AddressingMode7Bit, "\\_SB.PCI0.I2C1",
            0x00, ResourceConsumer, , Exclusive, )
        GpioInt (Level, ActiveLow, ExclusiveAndWake, PullUp, 0x0000, "\\_SB.GPIO", 0x00, ResourceConsumer, , )
        {
            0x0009
        }
        GpioIo (Exclusive, PullDefault, 0x0000, 0x0000, IoRestrictionOutputOnly, "\\_SB.GPI0", 0x00, ResourceConsumer, , )
        {
            0x0012, 0x0013
        }
    })
}
