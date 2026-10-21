// A laptop's power devices, for the virtual PC (check.py --power), added to its firmware's tables: a battery, an AC
// adapter and a lid. For the first 15 seconds after the battery is first read, it's charging at 64%, the PC plugged in
// and the lid open; after that it's unplugged, the battery discharging at 9% and the lid closed: the kernel's readings
// have to follow. What the kernel runs of the firmware's preparation for turning off (_TTS, _PTS) is written to QEMU's
// debug console at port 0x403, which the check reads.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "POWER", 1)
{
    // (QEMU's own DSDT has DBG and DBGB, for its port 0x402.)
    OperationRegion (CCDP, SystemIO, 0x0403, One)
    Field (CCDP, ByteAcc, NoLock, Preserve)
    {
        CCDB,   8
    }

    // A line on the debug console.
    Method (SAY, 1, Serialized)
    {
        Local0 = ToBuffer (Arg0)
        Local1 = Zero
        While ((Local1 < SizeOf (Local0)))
        {
            Local2 = DerefOf (Local0 [Local1])
            If ((Local2 == Zero))
            {
                Break
            }

            CCDB = Local2
            Local1++
        }

        CCDB = 0x0A
    }

    // When the battery was first read (the Timer counts 100 ns), and whether 15 seconds have gone by since.
    Name (FIRS, Zero)
    Method (LATE, 0, Serialized)
    {
        If ((FIRS == Zero))
        {
            FIRS = Timer
        }

        Return (((Timer - FIRS) > 150000000))
    }

    Method (_TTS, 1, NotSerialized)
    {
        SAY (Concatenate ("TTS ", ToDecimalString (Arg0)))
    }

    Method (_PTS, 1, NotSerialized)
    {
        SAY (Concatenate ("PTS ", ToDecimalString (Arg0)))
    }

    Scope (\_SB)
    {
        Device (BAT0)
        {
            Name (_HID, EisaId ("PNP0C0A"))
            Name (_UID, One)
            Method (_STA, 0, NotSerialized)
            {
                Return (0x1F)
            }

            // In mWh: 50 Wh designed, 48 Wh when last full; rechargeable, 11.4 V.
            Method (_BIF, 0, NotSerialized)
            {
                Return (Package (0x0D)
                {
                    Zero,
                    50000,
                    48000,
                    One,
                    11400,
                    4800,
                    1600,
                    100,
                    100,
                    "CC-TEST",
                    "42",
                    "LION",
                    "Chat Computer"
                })
            }

            // Charging at 12 W with 30.72 Wh in it (64%); later discharging at 9.6 W with 4.32 Wh (9%).
            Method (_BST, 0, NotSerialized)
            {
                If (LATE ())
                {
                    Return (Package (0x04)
                    {
                        One,
                        9600,
                        4320,
                        10800
                    })
                }

                Return (Package (0x04)
                {
                    0x02,
                    12000,
                    30720,
                    12300
                })
            }
        }

        Device (ADP1)
        {
            Name (_HID, "ACPI0003")
            Method (_PSR, 0, NotSerialized)
            {
                If (LATE ())
                {
                    Return (Zero)
                }

                Return (One)
            }
        }

        Device (LID0)
        {
            Name (_HID, EisaId ("PNP0C0D"))
            Method (_LID, 0, NotSerialized)
            {
                If (LATE ())
                {
                    Return (Zero)
                }

                Return (One)
            }
        }
    }
}
