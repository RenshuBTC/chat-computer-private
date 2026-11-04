// Methods: arguments passed by reference, objects a method makes for itself, scopes and devices, aliases, mutexes and
// events, recursion, loops that return, and the namespace's search rules.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "METHODS", 1)
{
    Method (P2, 2) { Local0 = Package (2) {}
                     Local0 [0] = Arg0
                     Local0 [1] = Arg1
                     Return (Local0) }
    Name (CNT1, 0)
    Name (VAL1, 5)
    Name (STR1, "abc")
    Name (BUF1, Buffer (4) { 9, 8, 7, 6 })
    Name (PKG1, Package () { 10, 20, 30 })
    Mutex (MUT1, 0)
    Event (EVT1)
    Alias (VAL1, AVL1)

    Scope (\_SB) {
        Device (DEV1) {
            Name (_HID, EisaId ("PNP0C0A"))
            Name (_UID, 1)
            Name (INNR, 0x77)
            Method (_STA) { Return (0x1F) }
            Method (_INI) { \CNT1++ }
            Method (WHO) { Return (INNR) }
            Device (SUB1) {
                Name (_ADR, 0x00010002)
                Method (UP) { Return (^^INNR) }
                Method (FAR) { Return (^^^DEV1.INNR) }
                Method (SRCH) { Return (VAL1) }
            }
        }
        Device (DEV2) {
            Name (_HID, "ACPI0003")
            Method (_STA) { Return (0) }
            Method (_INI) { \CNT1 += 100 }
            Device (HIDN) {
                Name (_ADR, 0)
                Method (_INI) { \CNT1 += 1000 }
            }
        }
    }

    Method (SET1, 1) { Arg0 = 0x55 }
    Method (INC1, 1) { Arg0++ }
    Method (MAKE, 1) {
        Name (TEMP, 0)
        TEMP = Arg0 * 2
        Return (TEMP)
    }
    Method (FIB, 1) {
        If (Arg0 < 2) { Return (Arg0) }
        Return (FIB (Arg0 - 1) + FIB (Arg0 - 2))
    }
    Method (FIND, 1) {
        Local0 = 0
        While (One) {
            If (DerefOf (PKG1 [Local0]) == Arg0) { Return (Local0) }
            Local0++
            If (Local0 >= SizeOf (PKG1)) { Return (Ones) }
        }
    }
    Method (NOTI) { Notify (\_SB.DEV1, 0x80) }

    Method (T200) { Return (CNT1) }
    Method (T201) { Return (\_SB.DEV1.WHO ()) }
    Method (T202) { Return (\_SB.DEV1.SUB1.UP ()) }
    Method (T203) { Return (\_SB.DEV1.SUB1.FAR ()) }
    Method (T204) { Return (\_SB.DEV1.SUB1.SRCH ()) }
    Method (T205) { Local0 = 1
                    SET1 (RefOf (Local0))
                    Return (Local0) }
    Method (T206) { SET1 (RefOf (VAL1))
                    Return (VAL1) }
    Method (T208) { Local0 = 7
                    INC1 (Local0)
                    Return (Local0) }
    Method (T209) { Return (P2 (MAKE (3), MAKE (4))) }
    Method (T210) { Return (FIB (15)) }
    Method (T211) { Return (P2 (FIND (20), FIND (99))) }
    Method (T212) { Return (AVL1) }
    Method (T213) { Return (Acquire (MUT1, 0xFFFF)) }
    Method (T214) { Acquire (MUT1, 0xFFFF)
                    Release (MUT1)
                    Return (1) }
    Method (T215) { Signal (EVT1)
                    Return (Wait (EVT1, 0)) }
    Method (T216) { Return (Wait (EVT1, 0)) }
    Method (T217) { Return (\_SB.DEV1._HID) }
    Method (T218) { Return (\_SB.DEV2._HID) }
    Method (T219) { Return (\_SB.DEV1.SUB1._ADR) }
    Method (T220) { Local0 = RefOf (PKG1)
                    Return (SizeOf (Local0)) }
    Method (T221) { Local0 = PKG1
                    Local0 [1] = 99
                    Return (P2 (DerefOf (Local0 [1]), DerefOf (PKG1 [1]))) }
    Method (T222) { Local0 = Index (PKG1, 2)
                    Return (DerefOf (Local0)) }
    Method (T223) { Store (Index (BUF1, 1), Local0)
                    Return (DerefOf (Local0)) }
    Method (T224) { Local0 = "\\_SB.DEV1.INNR"
                    Return (DerefOf (Local0)) }
    Method (T225) { CopyObject (STR1, Local0)
                    Return (Local0) }
    Method (T226) { STR1 = 0x41
                    Return (STR1) }
    Method (T227) { BUF1 = 0x1234
                    Return (BUF1) }
    Method (T228) { VAL1 = "12"
                    Return (VAL1) }
    Method (T229) { BUF1 = "ab"
                    Return (BUF1) }
    Method (T231) { Return (ObjectType (\_SB.DEV1)) }
    Method (T232) { Return (ObjectType (MUT1)) }
    Method (T233) { Return (ObjectType (EVT1)) }
    Method (T234) { Local0 = 5
                    Return (ObjectType (Local0)) }
    Method (T235) { Return (CondRefOf (\_SB.DEV2.HIDN)) }
    Method (T236) { Return (CondRefOf (\_SB.NONE.NONE)) }
    Method (T237) { If (CondRefOf (\_SB.DEV1.WHO, Local0)) { Return (DerefOf (Local0)) }
                    Return (0) }
    Method (T238) { Local1 = 0
                    For (Local0 = 0, Local0 < 5, Local0++) { Local1 += Local0 }
                    Return (Local1) }
    Method (T239) { Local0 = 2
                    Switch (Local0) {
                        Case (1) { Return (10) }
                        Case (Package () { 2, 3 }) { Return (20) }
                        Default { Return (30) }
                    } }
    Method (T240) { Local0 = "b"
                    Switch (Local0) {
                        Case ("a") { Return (1) }
                        Case ("b") { Return (2) }
                    }
                    Return (3) }
    Method (T241) { NOTI ()
                    Return (1) }
    Method (T242) { Return (P2 (Package () { 1, 2 }, "x")) }
    Method (FILL, 2) { Arg0 [0] = Arg1
                       Arg0 [1] = Arg1 + 1 }
    Method (T245) { Local0 = Package (2) {}
                    FILL (Local0, 5)
                    Return (Local0) }
    Method (T246) { FILL (PKG1, 7)
                    Return (PKG1) }
    Method (T247) { Local0 = Buffer (2) {}
                    FILL (Local0, 9)
                    Return (Local0) }
    Method (T243) { Local0 = Package () { Package () { 1, 2 }, 3 }
                    Local1 = DerefOf (Local0 [0])
                    Return (DerefOf (Local1 [1])) }
    // A method that returns nothing (here, with 0): called for what it does, fine; its value wanted, an error.
    Method (MAYB, 1) { If (Arg0) { Return (0x55) } }
    Method (T250) { MAYB (0)
                    Return (0x31) }
    Method (T251) { Local0 = MAYB (0)
                    Return (Local0) }
    Method (T252) { Return (MAYB (1)) }
    Method (T253) { Return (MAYB (0) + 1) }
    // A local nothing was stored in, read: an error.
    Method (T254) { If (0) { Local3 = 1 }
                    Return (Local3) }
    // Return (a call that returns nothing): nothing, not an error, until its value is wanted.
    Method (PASS, 1) { Return (MAYB (Arg0)) }
    Method (T255) { PASS (0)
                    Return (0x32) }
    Method (T256) { Return (PASS (1)) }
    Method (T257) { Local0 = PASS (0)
                    Return (Local0) }
}
