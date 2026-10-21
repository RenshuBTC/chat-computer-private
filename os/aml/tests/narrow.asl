// A DSDT of revision 1: integers are 32 bits.
DefinitionBlock ("", "DSDT", 1, "CHATCO", "NARROW", 1)
{
    Name (BUF1, Buffer () { 1, 2, 3, 4, 5, 6 })
    Method (T300) { Return (Ones) }
    Method (T301) { Return (0 - 1) }
    Method (T302) { Return (0xFFFFFFFF + 2) }
    Method (T303) { Return (ToBuffer (0x1234)) }
    Method (T304) { Return (Concatenate (1, 2)) }
    Method (T305) { Return (ToInteger (BUF1)) }
    Method (T306) { Return (ToHexString (0xAB)) }
    Method (T307) { Return (Concatenate ("x", 0xAB)) }
    Method (T308) { Return (~0) }
    Method (T309) { Return (1 << 35) }
    Method (T310) { Return (3 == 3) }
    Method (T311) { Return (FindSetLeftBit (0x80000000)) }
}
