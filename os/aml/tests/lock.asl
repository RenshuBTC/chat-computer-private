// The firmware's global lock: taken by Acquire (\_GL) and around fields with the Lock rule, once however deeply
// held, and given back when a method forgets to.
DefinitionBlock ("", "SSDT", 2, "CHATCO", "LOCK", 1)
{
    OperationRegion (RAM0, SystemMemory, 0x1000, 0x10)
    Field (RAM0, ByteAcc, Lock, Preserve)
    {
        LKD0, 8
    }
    Field (RAM0, ByteAcc, NoLock, Preserve)
    {
        Offset (0x01),
        FRE0, 8
    }
    Method (T001, 0, NotSerialized)
    {
        Acquire (\_GL, 0xFFFF)
        Local0 = LKD0
        Release (\_GL)
        Return (Local0)
    }
    Method (T002, 0, NotSerialized)
    {
        LKD0 = 0x05
        Return (LKD0)
    }
    Method (T003, 0, NotSerialized)
    {
        FRE0 = One
        Return (FRE0)
    }
    Method (T004, 0, NotSerialized)
    {
        Acquire (\_GL, 0xFFFF)
        Return (One)
    }
}
