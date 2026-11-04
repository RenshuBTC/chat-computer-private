#!/usr/bin/env python3
"""Build a synthetic Windows SYSTEM registry (regf) hive for testing the
Chat Computer Bluetooth key importer, WITHOUT any external tooling.

The hive is written to be parseable by regipy (the library the importer uses),
which models a hive as:
  * a 4096-byte REGF base block, then
  * 4096-byte HBIN blocks of cells;
  * the ROOT key is the FIRST cell of the first HBIN;
  * all intra-hive offsets are relative to (REGF_HEADER_SIZE); a cell is
    referenced by the offset of its 4-byte size field, and regipy reads a record
    at REGF_HEADER_SIZE + 4 + offset (i.e. just past the size field), consuming
    a 2-byte signature first.

We lay out exactly the keys the importer needs:
    \\Select\\Current = REG_DWORD 1          (-> ControlSet001)
    \\ControlSet001\\Services\\BTHPORT\\Parameters\\Keys\\<adapter>
        <device-mac> = REG_BINARY <16-byte link key>   (classic key)
        MasterIRK    = REG_BINARY <17 bytes>            (must be skipped)
    \\ControlSet001\\Services\\BTHPORT\\Parameters\\Devices\\<device-mac>
        Name = REG_BINARY <UTF-8 name, NUL-terminated>  (what Windows calls the device)

Usage: make_system_hive.py OUT.hive
"""
import struct
import sys

REGF_HEADER_SIZE = 4096
HBIN_SIZE = 4096 * 8  # plenty of room for our handful of cells


class HiveWriter:
    def __init__(self):
        # Cell region bytes (what goes after the HBIN header). Offsets we hand
        # out are relative to the hbins-start (i.e. file offset - 4096), and
        # point at the 4-byte cell size field.
        self.buf = bytearray()
        # The HBIN header is 32 bytes and sits at hbins-start; the first cell
        # therefore begins at hbins-relative offset 32.
        self.base = 32

    def _alloc(self, payload_with_sig: bytes) -> int:
        """Append a cell wrapping payload (which already starts with the 2-byte
        signature). Returns the hbins-relative offset of the cell's size field.
        """
        size = len(payload_with_sig) + 4
        pad = (8 - (size % 8)) % 8
        size += pad
        off = self.base + len(self.buf)
        self.buf += struct.pack("<i", -size)        # allocated (negative)
        self.buf += payload_with_sig
        self.buf += b"\x00" * pad
        return off

    # ---- cell builders ----------------------------------------------------
    def add_sk(self) -> int:
        # Minimal self-referential security cell.
        sd = bytes.fromhex("0100048000000000000000000000000014000000")
        # We must know our own offset for flink/blink; allocate then patch.
        payload = struct.pack("<2sHIIII", b"sk", 0, 0, 0, 1, len(sd)) + sd
        off = self._alloc(payload)
        # Patch flink/blink to self (offset of this cell).
        # Within buf, the cell starts at (off - self.base); payload begins +4.
        cstart = off - self.base
        struct.pack_into("<I", self.buf, cstart + 4 + 4, off)   # flink
        struct.pack_into("<I", self.buf, cstart + 4 + 8, off)   # blink
        return off

    def add_nk(self, name, sk_off, parent_off, subkeys=None, values=None,
               is_root=False):
        """subkeys: list[(name, nk_off)]; values: list[(name, vk_off)].
        Allocates subkey-list and value-list cells as needed, then the nk."""
        subkeys = subkeys or []
        values = values or []
        subkeys_off = 0xFFFFFFFF
        values_off = 0xFFFFFFFF
        if subkeys:
            subkeys_off = self._add_subkey_list(subkeys)
        if values:
            values_off = self._add_value_list(values)

        nm = name.encode("ascii")
        flags = 0x0020  # KEY_COMP_NAME (ASCII name)
        if is_root:
            flags |= 0x0004  # KEY_HIVE_ENTRY
        p = bytearray()
        p += b"nk"
        p += struct.pack("<H", flags)
        p += struct.pack("<Q", 0)                 # last_modified
        p += b"\x00\x00\x00\x00"                  # access_bits
        p += struct.pack("<I", parent_off & 0xFFFFFFFF)
        p += struct.pack("<I", len(subkeys))      # subkey_count
        p += struct.pack("<I", 0)                 # volatile_subkey_count
        p += struct.pack("<I", subkeys_off)       # subkeys_list_offset
        p += struct.pack("<I", 0xFFFFFFFF)        # volatile_subkeys_list_offset
        p += struct.pack("<I", len(values))       # values_count
        p += struct.pack("<I", values_off)        # values_list_offset
        p += struct.pack("<I", sk_off)            # security_key_offset
        p += struct.pack("<I", 0xFFFFFFFF)        # class_name_offset
        p += struct.pack("<I", 0)                 # largest_sk_name
        p += struct.pack("<I", 0)                 # largest_sk_class_name
        p += struct.pack("<I", 0)                 # largest_value_name
        p += struct.pack("<I", 0)                 # largest_value_data
        p += b"\x00\x00\x00\x00"                  # unknown
        p += struct.pack("<H", len(nm))           # key_name_size
        p += struct.pack("<H", 0)                 # class_name_size
        p += nm
        return self._alloc(bytes(p))

    def _add_subkey_list(self, subkeys):
        # Use a fast leaf (lf): signature 'lf', count, then (offset, 4-char hint)
        p = bytearray()
        p += b"lf"
        p += struct.pack("<H", len(subkeys))
        for name, off in subkeys:
            hint = name.encode("ascii")[:4].ljust(4, b"\x00")
            p += struct.pack("<I", off)
            p += hint
        return self._alloc(bytes(p))

    def _add_value_list(self, values):
        # A value list is just a cell holding an array of vk offsets.
        p = bytearray()
        for name, off in values:
            p += struct.pack("<I", off)
        return self._alloc(bytes(p))

    def add_vk(self, name, data, value_type):
        """Create a value (vk) cell. Returns its offset. Data is stored inline
        for small values (<=4 bytes packed) or in a separate data cell."""
        nm = name.encode("ascii")
        data_len = len(data)
        # Store data in a separate cell and reference it (simplest, uniform).
        data_off = self._alloc(data if data else b"\x00")
        p = bytearray()
        p += b"vk"
        p += struct.pack("<H", len(nm))           # name length
        p += struct.pack("<I", data_len)          # data size
        p += struct.pack("<I", data_off)          # data offset
        p += struct.pack("<I", value_type)        # type
        p += struct.pack("<H", 0x0001)            # flags: name is ASCII
        p += struct.pack("<H", 0)                 # spare
        p += nm
        return self._alloc(bytes(p))

    # ---- finalize ---------------------------------------------------------
    def finalize(self, root_off):
        # Fill the rest of the hbin with one free cell.
        body_max = HBIN_SIZE - 32
        if len(self.buf) > body_max:
            raise SystemExit("hive cells exceed HBIN size; enlarge HBIN_SIZE")
        free = body_max - len(self.buf)
        if free >= 8:
            self.buf += struct.pack("<i", free) + b"\x00" * (free - 4)
        else:
            self.buf += b"\x00" * free

        hbin = bytearray()
        hbin += b"hbin"
        hbin += struct.pack("<I", 0)              # this hbin's offset
        hbin += struct.pack("<I", HBIN_SIZE)      # size
        hbin += struct.pack("<Q", 0)
        hbin += struct.pack("<Q", 0)
        hbin += struct.pack("<I", 0)
        hbin += bytes(self.buf)

        base = bytearray(REGF_HEADER_SIZE)
        struct.pack_into("<4s", base, 0, b"regf")
        struct.pack_into("<I", base, 4, 1)        # primary seq
        struct.pack_into("<I", base, 8, 1)        # secondary seq
        struct.pack_into("<Q", base, 12, 0)
        struct.pack_into("<I", base, 20, 1)       # major
        struct.pack_into("<I", base, 24, 5)       # minor
        struct.pack_into("<I", base, 28, 0)       # file type primary
        struct.pack_into("<I", base, 32, 1)       # file format
        struct.pack_into("<I", base, 36, root_off)
        struct.pack_into("<I", base, 40, HBIN_SIZE)
        struct.pack_into("<I", base, 44, 1)
        name = "SYSTEM".encode("utf-16-le")
        base[48:48 + len(name)] = name
        csum = 0
        for i in range(0, 508, 4):
            csum ^= struct.unpack_from("<I", base, i)[0]
        if csum == 0xFFFFFFFF:
            csum = 0xFFFFFFFE
        if csum == 0:
            csum = 1
        struct.pack_into("<I", base, 508, csum)
        return bytes(base) + bytes(hbin)


# Default synthetic contents (also importable by tests).
DEFAULT_ADAPTER = "0a1b2c3d4e5f"          # -> 0A:1B:2C:3D:4E:5F
DEFAULT_DEVICES = {
    # device-mac-hex : 16-byte classic link key (hex)
    "a1b2c3d4e5f6": "0123456789abcdef0123456789abcdef",  # -> A1:B2:C3:D4:E5:F6
    "c0ffee123456": "fedcba9876543210fedcba9876543210",  # -> C0:FF:EE:12:34:56
}
# A non-16-byte blob the importer must skip.
SKIP_VALUE = ("MasterIRK", "00112233445566778899aabbccddeeff00")
# What Windows calls the devices (Devices\\<mac>\\Name). The second device has none.
DEFAULT_NAMES = {"a1b2c3d4e5f6": "Sam\u2019s AirPods Pro"}


def build(out_path, adapter=DEFAULT_ADAPTER, devices=None, names=None):
    """Correct single-pass builder: root cell FIRST (regipy requirement), with
    its subkey/value list cells and all descendants allocated AFTER it, and the
    root's subkey-list offset patched in once children exist."""
    REG_DWORD_V = 4
    REG_BINARY_V = 3
    devices = devices or DEFAULT_DEVICES
    names = DEFAULT_NAMES if names is None else names

    w = HiveWriter()

    # 1) Allocate the root nk FIRST as a placeholder with a subkey list we will
    #    patch. We reserve the root with a known subkey count (2) and a
    #    subkeys_list_offset we backfill.
    # To do this we need the subkey-list cell to exist; allocate it first but
    # AFTER root? regipy only needs the root to be the FIRST cell. So: allocate
    # root nk cell first with a placeholder subkeys_off, remember where to patch.
    root_name = b"ROOT"
    root_flags = 0x0020 | 0x0004
    root_payload = bytearray()
    root_payload += b"nk"
    root_payload += struct.pack("<H", root_flags)
    root_payload += struct.pack("<Q", 0)
    root_payload += b"\x00\x00\x00\x00"
    root_payload += struct.pack("<I", 0xFFFFFFFF)   # parent
    root_payload += struct.pack("<I", 2)            # subkey_count (Select, CS001)
    root_payload += struct.pack("<I", 0)
    root_payload += struct.pack("<I", 0xFFFFFFFF)   # subkeys_list_offset (patch)
    root_payload += struct.pack("<I", 0xFFFFFFFF)
    root_payload += struct.pack("<I", 0)            # values_count
    root_payload += struct.pack("<I", 0xFFFFFFFF)   # values_list_offset
    root_payload += struct.pack("<I", 0xFFFFFFFF)   # sk (patch)
    root_payload += struct.pack("<I", 0xFFFFFFFF)   # class
    root_payload += struct.pack("<I", 0) * 0
    root_payload += struct.pack("<IIII", 0, 0, 0, 0)
    root_payload += b"\x00\x00\x00\x00"
    root_payload += struct.pack("<H", len(root_name))
    root_payload += struct.pack("<H", 0)
    root_payload += root_name
    root_off = w._alloc(bytes(root_payload))
    root_cstart = root_off - w.base

    # 2) sk
    sk = w.add_sk()
    # patch root sk offset
    struct.pack_into("<I", w.buf, root_cstart + 4 + 2 + 2 + 8 + 4 + 4 + 4 + 4 + 4 + 4 + 4 + 4, sk)

    # 3) children
    val_pairs = []
    for dev_hex, key_hex in devices.items():
        off = w.add_vk(dev_hex, bytes.fromhex(key_hex), REG_BINARY_V)
        val_pairs.append((dev_hex, off))
    skip_off = w.add_vk(SKIP_VALUE[0], bytes.fromhex(SKIP_VALUE[1]), REG_BINARY_V)
    val_pairs.append((SKIP_VALUE[0], skip_off))

    adapter_off = w.add_nk(adapter, sk, root_off, values=val_pairs)
    keys_off = w.add_nk("Keys", sk, root_off, subkeys=[(adapter, adapter_off)])
    device_keys = []
    for dev_hex, name in sorted(names.items()):
        name_off = w.add_vk("Name", name.encode("utf-8") + b"\x00", REG_BINARY_V)
        device_keys.append((dev_hex, w.add_nk(dev_hex, sk, root_off, values=[("Name", name_off)])))
    params_subkeys = [("Keys", keys_off)]
    if device_keys:
        params_subkeys.insert(0, ("Devices", w.add_nk("Devices", sk, root_off, subkeys=device_keys)))
    params_off = w.add_nk("Parameters", sk, root_off, subkeys=params_subkeys)
    bthport_off = w.add_nk("BTHPORT", sk, root_off, subkeys=[("Parameters", params_off)])
    services_off = w.add_nk("Services", sk, root_off, subkeys=[("BTHPORT", bthport_off)])
    cs1_off = w.add_nk("ControlSet001", sk, root_off, subkeys=[("Services", services_off)])

    cur_off = w.add_vk("Current", struct.pack("<I", 1), REG_DWORD_V)
    def_off = w.add_vk("Default", struct.pack("<I", 1), REG_DWORD_V)
    select_off = w.add_nk("Select", sk, root_off,
                          values=[("Current", cur_off), ("Default", def_off)])

    # 4) root subkey list (Select + ControlSet001), then patch root.
    root_subkeys = [("Select", select_off), ("ControlSet001", cs1_off)]
    root_sklist_off = w._add_subkey_list(root_subkeys)
    # subkeys_list_offset field position inside root payload:
    # 2(sig)+2(flags)+8(time)+4(access)+4(parent)+4(subcount)+4(volcount) = 28
    struct.pack_into("<I", w.buf, root_cstart + 4 + 28, root_sklist_off)

    data = w.finalize(root_off)
    with open(out_path, "wb") as f:
        f.write(data)
    return out_path


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "system_test.hive"
    build(out)
    print("wrote", out)
