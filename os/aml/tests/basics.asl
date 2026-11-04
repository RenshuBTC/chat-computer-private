// Constants, arithmetic, logic, conversions, strings, buffers and packages: each T method's result is compared with
// what ACPICA's acpiexec returns for it.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "BASICS", 1)
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
    Name (INT1, 0x1234)
    Name (STR1, "Hello")
    Name (BUF1, Buffer (6) { 1, 0xAB, 3 })
    Name (PKG1, Package () { 1, "two", Buffer () { 3, 4 }, Package () { 5, "six" } })
    Name (PKG2, Package (5) { 7 })
    Method (ADD2, 2) { Return (Arg0 + Arg1) }
    Method (FACT, 1) {
        If (Arg0 <= 1) { Return (1) }
        Return (Arg0 * FACT (Arg0 - 1))
    }

    Method (T000) { Return (Zero) }
    Method (T001) { Return (One) }
    Method (T002) { Return (Ones) }
    Method (T003) { Return (0x12) }
    Method (T004) { Return (0x1234) }
    Method (T005) { Return (0x12345678) }
    Method (T006) { Return (0x123456789ABCDEF0) }
    Method (T007) { Return ("a string") }
    Method (T008) { Return (BUF1) }
    Method (T009) { Return (PKG1) }
    Method (T010) { Return (PKG2) }
    Method (T011) { Return (ADD2 (3, 4)) }
    Method (T012) { Return (FACT (10)) }
    Method (T013) { Return (0x10 - 0x20) }
    Method (T014) { Return (7 * 6) }
    Method (T015) { Return (100 / 7) }
    Method (T016) { Return (100 % 7) }
    Method (T017) { Return (1 << 40) }
    Method (T018) { Return (0xF0 >> 4) }
    Method (T019) { Return (0xF0 & 0x3C) }
    Method (T020) { Return (0xF0 | 0x0F) }
    Method (T021) { Return (0xF0 ^ 0xFF) }
    Method (T022) { Return (~0x0F) }
    Method (T023) { Return (NAnd (0xF0, 0x3C)) }
    Method (T024) { Return (NOr (0xF0, 0x0F)) }
    Method (T025) { Return (FindSetLeftBit (0x80)) }
    Method (T026) { Return (FindSetRightBit (0x80)) }
    Method (T027) { Return (FindSetLeftBit (0)) }
    Method (T028) { Return (ToBCD (1234)) }
    Method (T029) { Return (FromBCD (0x1234)) }
    Method (T030) { Divide (17, 5, Local0, Local1)
                    Return (Local0) }
    Method (T031) { Divide (17, 5, Local0, Local1)
                    Return (Local1) }
    Method (T032) { Local0 = 5
                    Local0++
                    Local0++
                    Local0--
                    Return (Local0) }
    Method (T033) { Return (LAnd (1, 0)) }
    Method (T034) { Return (LOr (1, 0)) }
    Method (T035) { Return (LNot (0)) }
    Method (T036) { Return (3 == 3) }
    Method (T037) { Return (3 != 3) }
    Method (T038) { Return (3 < 4) }
    Method (T039) { Return (3 > 4) }
    Method (T040) { Return (3 <= 3) }
    Method (T041) { Return (3 >= 4) }
    Method (T042) { Return ("abc" == "abc") }
    Method (T043) { Return ("abc" < "abd") }
    Method (T044) { Return ("abc" > "ab") }
    Method (T045) { Return (Buffer () { 1, 2 } == Buffer () { 1, 2 }) }
    Method (T046) { Return (Concatenate ("x", 0x1234)) }
    Method (T047) { Return (Concatenate ("x", BUF1)) }
    Method (T048) { Return (Concatenate (0x12, 0x34)) }
    Method (T049) { Return (Concatenate (BUF1, "q")) }
    Method (T050) { Return (Concatenate ("ab", "cd")) }
    Method (T051) { Return (ToHexString (0x1234)) }
    Method (T052) { Return (ToHexString (BUF1)) }
    Method (T053) { Return (ToDecimalString (BUF1)) }
    Method (T054) { Return (ToDecimalString (1234)) }
    Method (T055) { Return (ToInteger ("0x1F")) }
    Method (T056) { Return (ToInteger ("123")) }
    Method (T057) { Return (ToInteger (BUF1)) }
    Method (T058) { Return (ToBuffer ("ab")) }
    Method (T059) { Return (ToBuffer (0x1234)) }
    Method (T060) { Return (ToString (Buffer () { 0x41, 0x42, 0, 0x43 }, Ones)) }
    Method (T061) { Return (ToString (Buffer () { 0x41, 0x42, 0x43 }, 2)) }
    Method (T062) { Local0 = "1F"
                    Return (Local0 + 1) }
    Method (T063) { Return (Mid ("hello", 1, 3)) }
    Method (T064) { Return (Mid (Buffer () { 1, 2, 3, 4 }, 2, 9)) }
    Method (T065) { Return (SizeOf (STR1)) }
    Method (T066) { Return (SizeOf (BUF1)) }
    Method (T067) { Return (SizeOf (PKG1)) }
    Method (T068) { Return (ObjectType (INT1)) }
    Method (T069) { Return (ObjectType (STR1)) }
    Method (T070) { Return (ObjectType (BUF1)) }
    Method (T071) { Return (ObjectType (PKG1)) }
    Method (T072) { Return (ObjectType (ADD2)) }
    Method (T073) { Return (DerefOf (PKG1 [1])) }
    Method (T074) { Return (DerefOf (BUF1 [1])) }
    Method (T075) { Return (Match (Package () { 1, 5, 7, 9 }, MGT, 4, MLT, 8, 0)) }
    Method (T076) { Return (Match (Package () { 1, 5, 7, 9 }, MEQ, 3, MTR, 0, 0)) }
    Method (T077) { Store (PKG1, Local0)
                    Local0 [0] = 99
                    Return (P2 (DerefOf (Local0 [0]), DerefOf (PKG1 [0]))) }
    Method (T078) { PKG2 [1] = "set"
                    Return (PKG2) }
    Method (T079) { BUF1 [5] = 0x55
                    Return (BUF1) }
    Method (T080) { Local0 = Buffer (3) {}
                    Local0 [1] = 7
                    Return (Local0) }
    Method (T081) { Local0 = 0
                    Local1 = 0
                    While (Local0 < 10) {
                        Local0++
                        If (Local0 == 3) { Continue }
                        If (Local0 == 8) { Break }
                        Local1 += Local0
                    }
                    Return (Local1) }
    Method (T082) { If (0) { Return (1) } ElseIf (1) { Return (2) } Else { Return (3) } }
    Method (T083) { Local0 = RefOf (INT1)
                    Return (DerefOf (Local0)) }
    Method (T084) { Return (CondRefOf (\NONE)) }
    Method (T085) { Return (CondRefOf (\INT1)) }
    Method (T086) { Return (CondRefOf (\INT1, Local0)) }
    Method (T087) { Local0 = Package (3) { 1 }
                    Return (SizeOf (Local0)) }
    Method (T088) { Local0 = 3
                    Local1 = Package (Local0) { 4, 5 }
                    Return (Local1) }
    Method (T089) { Return (\_OSI ("Windows 2015")) }
    Method (T090) { Return (\_OSI ("Linux")) }
    Method (T091) { Return (\_REV) }
    Method (T092) { Return (\_OS) }
    Method (T093) { Local0 = Timer
                    Return (Local0 > 0) }
    Method (T094) { Return (ShiftLeft (1, 64)) }
    Method (T095) { Return (Buffer (2) { 1, 2, 3 }) }
    Method (T096) { Return (Buffer (4) { 1 }) }
    Method (T097) { Return (ToHexString ("abc")) }
    Method (T098) { Return (ToDecimalString ("abc")) }
    Method (T099) { Return (Concatenate (BUF1, 0x41)) }
}
