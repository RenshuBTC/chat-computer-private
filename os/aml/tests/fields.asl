// Operation regions and their fields (simulated memory, as acpiexec simulates it), buffer fields, index and bank
// fields, update rules, and access widths.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "FIELDS", 1)
{
    // Packages of values (a package's elements can't be expressions: names in one are references).
    Method (P2, 2) { Local0 = Package (2) {}
                     Local0 [0] = Arg0
                     Local0 [1] = Arg1
                     Return (Local0) }
    Method (P3, 3) { Local0 = Package (3) {}
                     Local0 [0] = Arg0
                     Local0 [1] = Arg1
                     Local0 [2] = Arg2
                     Return (Local0) }
    Method (P5, 5) { Local0 = Package (5) {}
                     Local0 [0] = Arg0
                     Local0 [1] = Arg1
                     Local0 [2] = Arg2
                     Local0 [3] = Arg3
                     Local0 [4] = Arg4
                     Return (Local0) }
    Method (P6, 6) { Local0 = Package (6) {}
                     Local0 [0] = Arg0
                     Local0 [1] = Arg1
                     Local0 [2] = Arg2
                     Local0 [3] = Arg3
                     Local0 [4] = Arg4
                     Local0 [5] = Arg5
                     Return (Local0) }
    OperationRegion (MEM1, SystemMemory, 0x10000000, 0x100)
    Field (MEM1, ByteAcc, NoLock, Preserve) {
        BYT0, 8,
        BYT1, 8,
        WRD1, 16,
        , 4,
        NIB1, 4,
        BIT1, 1,
        BIT2, 1,
        , 6,
        Offset (0x10),
        DW10, 32,
        QW14, 64,
        BIG1, 128,
    }
    Field (MEM1, DWordAcc, NoLock, WriteAsOnes) {
        Offset (0x40),
        PRT1, 4,
        PRT2, 12,
    }
    Field (MEM1, AnyAcc, NoLock, WriteAsZeros) {
        Offset (0x50),
        ANY1, 3,
        ANY2, 9,
        Offset (0x58),
        ANY3, 32,
    }
    OperationRegion (IDXR, SystemMemory, 0x10001000, 0x2)
    Field (IDXR, ByteAcc, NoLock, Preserve) {
        INDX, 8,
        DATA, 8,
    }
    IndexField (INDX, DATA, ByteAcc, NoLock, Preserve) {
        IF00, 8,
        IF01, 8,
        Offset (0x10),
        IF10, 16,
    }
    OperationRegion (BNKR, SystemMemory, 0x10002000, 0x10)
    Field (BNKR, ByteAcc, NoLock, Preserve) {
        BANK, 8,
    }
    BankField (BNKR, BANK, 2, ByteAcc, NoLock, Preserve) {
        Offset (0x4),
        BK20, 8,
    }
    Name (BUFF, Buffer (16) { 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88 })
    CreateByteField (BUFF, 1, CBY1)
    CreateWordField (BUFF, 2, CWD2)
    CreateDWordField (BUFF, 4, CDW4)
    CreateQWordField (BUFF, 8, CQW8)
    CreateBitField (BUFF, 9, CBT9)
    CreateField (BUFF, 4, 12, CF12)
    CreateField (BUFF, 0, 96, CF96)

    Method (T100) { BYT0 = 0x12
                    BYT1 = 0x34
                    Return (WRD1) }
    Method (T101) { BYT0 = 0x12
                    BYT1 = 0x34
                    WRD1 = 0xBEEF
                    Return (P3 (BYT0, BYT1, WRD1)) }
    Method (T102) { NIB1 = 0xF
                    BIT2 = 1
                    Return (P3 (NIB1, BIT1, BIT2)) }
    Method (T103) { DW10 = 0xDEADBEEF
                    QW14 = 0x0123456789ABCDEF
                    Return (P2 (DW10, QW14)) }
    Method (T104) { BIG1 = Buffer () { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 }
                    Return (BIG1) }
    Method (T105) { PRT1 = 0x5
                    PRT2 = 0x123
                    Return (P2 (PRT1, PRT2)) }
    Method (T106) { ANY1 = 5
                    ANY2 = 0x1FF
                    ANY3 = 0x01020304
                    Return (P3 (ANY1, ANY2, ANY3)) }
    Method (T107) { IF01 = 0xAA
                    IF10 = 0xBEEF
                    Return (P5 (IF00, IF01, IF10, INDX, DATA)) }
    Method (T108) { BK20 = 0x77
                    Return (P2 (BK20, BANK)) }
    Method (T109) { Return (P6 (CBY1, CWD2, CDW4, CQW8, CBT9, CF12)) }
    Method (T110) { CWD2 = 0xABCD
                    CBT9 = 0
                    Return (BUFF) }
    Method (T111) { Return (CF96) }
    Method (T112) { CF12 = 0xFFF
                    Return (BUFF) }
    Method (T113) { Local0 = Buffer (8) { 1, 2, 3, 4, 5, 6, 7, 8 }
                    CreateDWordField (Local0, 2, LFLD)
                    LFLD = 0x99999999
                    Return (Local0) }
    Method (T114) { Return (ObjectType (BYT0)) }
    Method (T115) { Return (ObjectType (CBY1)) }
    Method (T116) { Return (ObjectType (MEM1)) }
}
