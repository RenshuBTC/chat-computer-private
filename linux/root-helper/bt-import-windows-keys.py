#!/usr/bin/env python3
"""Import Bluetooth Classic (BR/EDR) link keys from an offline Windows SYSTEM
registry hive into BlueZ, so a device paired under Windows (e.g. AirPods)
reconnects under Chat Computer without re-pairing.

Run by the root helper as:
    /opt/chatcomputer/venv/bin/python -I bt-import-windows-keys.py \
        --hive /mnt/windows/Windows/System32/config/SYSTEM \
        --bluetooth-dir /var/lib/bluetooth

Prints ONE JSON object to stdout describing what was imported. Exit 0 on
success (even if zero keys matched), non-zero only on hard errors.

--------------------------------------------------------------------------
Format research (cited so the byte order is auditable):

* Windows stores classic link keys under
    SYSTEM\\ControlSet00N\\Services\\BTHPORT\\Parameters\\Keys\\<adapter>
  where <adapter> is the adapter's MAC with no separators, lowercase
  (e.g. "0a1b2c3d4e5f"). Each REG_BINARY value under it is named by the
  *device* MAC (same 12-hex-lowercase form) and holds the 16-byte link key.
  Source: Microsoft "Bluetooth registry entries" (BTHPORT\\Parameters\\Keys),
  and the x2es/bt-dualboot project (bt_dualboot/bt_windows/devices.py,
  windows_registry.py) which parses exactly these keys.

* The active control set is selected by SYSTEM\\Select\\Current (a REG_DWORD,
  e.g. 1 -> ControlSet001). Source: regipy's active_controlset plugin and the
  standard Windows registry layout.

* BlueZ stores the same key, byte-for-byte in the SAME order (no reversal),
  as uppercase hex in:
    /var/lib/bluetooth/<ADAPTER>/<DEVICE>/info
  under a [LinkKey] section:
    [LinkKey]
    Key=<32 uppercase hex chars>
    Type=4
    PINLength=0
  Type=4 is HCI_LINK_KEY_TYPE "authenticated combination key (P-256)"/
  unauthenticated combination — BlueZ accepts 0..8; 4 is what desktop stacks
  write for a normally-paired device, and is what bt-dualboot's own test
  fixtures (tests/bt_linux/data_samples/.../info) contain. PINLength=0 for a
  non-PIN (SSP) pairing. MAC bytes and key bytes are NOT reversed between
  Windows and BlueZ for classic keys — the 12-hex value-name maps directly to
  AA:BB:CC:DD:EE:FF and the 16 key bytes copy straight across. (bt-dualboot
  copies them verbatim; see hex_string_from_reg + the info writer.)

* What Windows calls each paired device is under
    SYSTEM\\ControlSet00N\\Services\\BTHPORT\\Parameters\\Devices\\<device>
  (the same 12-hex form) in a value named Name: REG_BINARY, the name in UTF-8,
  ending in a NUL. It goes into the info file's [General] section with
  Trusted=true and SupportedTechnologies=BR/EDR;, the way bluetoothd writes a
  device it paired itself (bt-dualboot's fixtures, copied from real BlueZ info
  files, have exactly these). Without a name BlueZ lists the device by its
  address, and Chat Computer couldn't tell AirPods were among those copied over: it
  asked to pair them again instead of connecting them.
--------------------------------------------------------------------------
"""

import argparse
import configparser
import io
import json
import os
import re
import sys

MAC_VALUE_RE = re.compile(r"^[0-9a-fA-F]{12}$")


def mac_from_hex(h: str) -> str:
    """'0a1b2c3d4e5f' -> '0A:1B:2C:3D:4E:5F' (uppercase, colon-separated)."""
    h = h.upper()
    return ":".join(h[i : i + 2] for i in range(0, 12, 2))


def key_to_bluez_hex(data: bytes) -> str:
    """16 raw key bytes -> 32 uppercase hex chars, same byte order."""
    return "".join("%02X" % b for b in data)


def get_current_controlset(hive):
    """Return the control-set key path string, e.g. '\\ControlSet001'.

    Reads \\Select\\Current; falls back to ControlSet001 if unreadable.
    """
    try:
        sel = hive.get_key(r"\Select")
        for v in sel.iter_values():
            if v.name == "Current":
                n = int(v.value)
                return r"\ControlSet%03d" % n
    except Exception:
        pass
    return r"\ControlSet001"


def iter_adapter_keys(hive, controlset):
    """Yield (adapter_mac_hex, NKRecord) for each adapter under BTHPORT Keys."""
    path = controlset + r"\Services\BTHPORT\Parameters\Keys"
    keys = hive.get_key(path)  # raises if absent
    for sub in keys.iter_subkeys():
        name = sub.name
        if MAC_VALUE_RE.match(name):
            yield name, sub


def extract_value_bytes(value) -> bytes:
    """Normalise a regipy value payload to raw bytes.

    regipy 6 returns REG_BINARY as a hex string ("0123ab..."); older versions
    gave `bytes`. Lists of ints are taken too, just in case.
    """
    v = value.value
    if isinstance(v, bytes):
        return v
    if isinstance(v, bytearray):
        return bytes(v)
    if isinstance(v, str):
        s = v.strip().replace(" ", "").replace(":", "")
        # Could be hex; try to decode.
        try:
            return bytes.fromhex(s)
        except ValueError:
            return v.encode("latin-1", "ignore")
    if isinstance(v, (list, tuple)):
        try:
            return bytes(int(x) & 0xFF for x in v)
        except Exception:
            return b""
    return b""


def read_adapter_links(adapter_key):
    """Return {device_mac_hex: key_bytes} for a BTHPORT adapter subkey.

    Only 16-byte values named like a MAC are treated as classic link keys;
    anything else (MasterIRK, LTK blobs, CentralIRK, etc.) is skipped so we
    only ever import classic keys.
    """
    out = {}
    for v in adapter_key.iter_values():
        if not v.name or not MAC_VALUE_RE.match(v.name):
            continue
        data = extract_value_bytes(v)
        if len(data) != 16:
            # Classic combination keys are exactly 16 bytes. Skip LE/other.
            continue
        out[v.name.lower()] = data
    return out


def name_from_value(value) -> str:
    """The device name in a Devices\\<mac>\\Name value: UTF-8 up to the first NUL (or a string), printable
    characters only (it goes on one line of an INI file), at most the 248 bytes Bluetooth names have."""
    v = value.value
    kind = str(getattr(value, "value_type", "") or "")
    if isinstance(v, str) and "BINARY" not in kind:
        text = v                        # REG_SZ: the name itself
    elif isinstance(v, (str, bytes, bytearray, list, tuple)):
        # REG_BINARY, which regipy hands over as hex ("52656e..."), or as bytes.
        text = extract_value_bytes(value).split(b"\x00", 1)[0].decode("utf-8", "replace")
    else:
        return ""
    text = "".join(ch for ch in text if ch.isprintable()).strip()
    while len(text.encode("utf-8")) > 248:
        text = text[:-1]
    return text


def read_device_names(hive, controlset):
    """{device_mac_hex: name}: what Windows calls each paired device. Devices without a readable name are left
    out (the import still works; BlueZ shows them by address until it learns their name)."""
    out = {}
    try:
        devices = hive.get_key(controlset + r"\Services\BTHPORT\Parameters\Devices")
        subkeys = list(devices.iter_subkeys())
    except Exception:
        return out
    for sub in subkeys:
        if not MAC_VALUE_RE.match(sub.name or ""):
            continue
        try:
            for v in sub.iter_values():
                if v.name == "Name":
                    name = name_from_value(v)
                    if name:
                        out[sub.name.lower()] = name
                    break
        except Exception:
            continue
    return out


def _fields(cp):
    """What this importer sets in an info file, to tell whether a write would change anything."""
    get = lambda sec, key: cp.get(sec, key, fallback=None) if cp.has_section(sec) else None  # noqa: E731
    return tuple(get(s, k) for s, k in (("LinkKey", "Key"), ("LinkKey", "Type"), ("LinkKey", "PINLength"),
                                         ("General", "Name"), ("General", "Trusted"),
                                         ("General", "SupportedTechnologies")))


def update_info_file(info_path: str, key_hex: str, name: str = "") -> str:
    """Write/refresh the [LinkKey] section in a BlueZ device info file, and give the device its name from Windows
    and BlueZ's trust when it has neither (values already there are kept: the person may have renamed it here),
    preserving everything else. Returns 'created', 'updated' or 'unchanged'.
    """
    # No interpolation: a "%" in a device's name is just a "%".
    cp = configparser.ConfigParser(interpolation=None)
    # BlueZ info files are case-sensitive in their keys; preserve case.
    cp.optionxform = str
    existed = os.path.exists(info_path)
    if existed:
        # Read with a permissive parser; BlueZ files are INI-compatible.
        with io.open(info_path, "r", encoding="utf-8") as fh:
            cp.read_file(fh)

    before = _fields(cp)
    if not cp.has_section("General"):
        cp.add_section("General")
    general = cp["General"]
    if name and not general.get("Name"):
        general["Name"] = name
    general.setdefault("SupportedTechnologies", "BR/EDR;")
    general.setdefault("Trusted", "true")        # it may connect to Chat Computer by itself, as it did to Windows
    general.setdefault("Blocked", "false")
    if not cp.has_section("LinkKey"):
        cp.add_section("LinkKey")

    cp.set("LinkKey", "Key", key_hex)
    cp.set("LinkKey", "Type", "4")
    cp.set("LinkKey", "PINLength", "0")

    if existed and _fields(cp) == before:
        return "unchanged"

    # Readable by root only, as bluetoothd keeps them (folders 0700, info files 0600): the link key lets
    # anything pose as the paired device.
    os.makedirs(os.path.dirname(info_path), mode=0o700, exist_ok=True)
    # Write atomically-ish: temp then rename.
    tmp = info_path + ".chatcomputer.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)  # also when an earlier run left the temp file behind
    with io.open(fd, "w", encoding="utf-8") as fh:
        cp.write(fh, space_around_delimiters=False)
    os.replace(tmp, info_path)
    return "created" if not existed else "updated"


def main(argv=None):
    ap = argparse.ArgumentParser(description="Import Windows BT link keys into BlueZ")
    ap.add_argument("--hive", required=True, help="path to Windows SYSTEM hive")
    ap.add_argument(
        "--bluetooth-dir",
        default="/var/lib/bluetooth",
        help="BlueZ state dir (default /var/lib/bluetooth)",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="parse and report, but do not write info files",
    )
    args = ap.parse_args(argv)

    result = {
        "hive": args.hive,
        "controlset": None,
        "adapters": [],
        "imported": [],
        "skipped_no_adapter_dir": [],
        "errors": [],
    }

    try:
        from regipy.registry import RegistryHive
    except Exception as e:  # pragma: no cover - import guard
        print(json.dumps({"ok": False, "error": "regipy not available: %s" % e}))
        return 2

    if not os.path.isfile(args.hive):
        print(json.dumps({"ok": False, "error": "hive not found: %s" % args.hive}))
        return 2

    try:
        hive = RegistryHive(args.hive)
    except Exception as e:
        print(json.dumps({"ok": False, "error": "cannot open hive: %s" % e}))
        return 2

    controlset = get_current_controlset(hive)
    result["controlset"] = controlset
    names = read_device_names(hive, controlset)

    try:
        adapters = list(iter_adapter_keys(hive, controlset))
    except Exception as e:
        # No BTHPORT keys at all -> nothing to import, but not a hard error.
        print(
            json.dumps(
                {
                    "ok": True,
                    "controlset": controlset,
                    "imported": [],
                    "note": "no BTHPORT\\Parameters\\Keys (%s)" % e,
                }
            )
        )
        return 0

    # Map existing BlueZ adapters so we only import for adapters we actually
    # have locally (the hardware MAC should match; BlueZ dir names are MACs).
    local_adapters = set()
    if os.path.isdir(args.bluetooth_dir):
        for name in os.listdir(args.bluetooth_dir):
            if re.match(r"^[0-9A-Fa-f:]{17}$", name):
                local_adapters.add(name.upper())

    for adapter_hex, adapter_key in adapters:
        adapter_mac = mac_from_hex(adapter_hex)
        result["adapters"].append(adapter_mac)
        try:
            links = read_adapter_links(adapter_key)
        except Exception as e:
            result["errors"].append("adapter %s: %s" % (adapter_mac, e))
            continue

        for dev_hex, key_bytes in links.items():
            dev_mac = mac_from_hex(dev_hex)
            key_hex = key_to_bluez_hex(key_bytes)
            info_path = os.path.join(args.bluetooth_dir, adapter_mac, dev_mac, "info")
            adapter_dir = os.path.join(args.bluetooth_dir, adapter_mac)

            # Only import if this adapter exists locally, OR its directory
            # already exists (device previously seen). If the adapter dir is
            # absent we still create it when the adapter MAC matches local
            # hardware; otherwise we record it as skipped (a key for a BT
            # adapter not in this machine is useless and we avoid littering).
            if (
                adapter_mac not in local_adapters
                and not os.path.isdir(adapter_dir)
            ):
                result["skipped_no_adapter_dir"].append(
                    {"adapter": adapter_mac, "device": dev_mac}
                )
                continue

            if args.dry_run:
                result["imported"].append(
                    {"adapter": adapter_mac, "device": dev_mac, "action": "dry-run"}
                )
                continue

            try:
                action = update_info_file(info_path, key_hex, names.get(dev_hex, ""))
                result["imported"].append(
                    {"adapter": adapter_mac, "device": dev_mac, "action": action,
                     "name": names.get(dev_hex, "")}
                )
            except Exception as e:
                result["errors"].append("%s/%s: %s" % (adapter_mac, dev_mac, e))

    result["ok"] = True
    result["count"] = len(result["imported"])
    # How many info files this run wrote: bluetoothd is restarted (dropping what's connected for a moment) only
    # when it has something new to read.
    result["changed"] = sum(1 for i in result["imported"] if i.get("action") in ("created", "updated"))
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
