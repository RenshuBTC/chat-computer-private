"""The Mac app's main program (mac/launcher/launcher.c), built for Linux here. It reads launch.conf next to
it, sends what ClaudeOS prints to ~/ClaudeOS/logs/app.log, loads the shared Python library and runs
`python -m claudeos run` inside its own process, in ClaudeOS's Python environment. A stand-in ClaudeOS
reports how it was started. (The Mac-only parts, finding its own path with _NSGetExecutablePath and the
alert window, can only run on a Mac; the Mac builds are checked for their shape.)"""
import json
import os
import plistlib
import shutil
import struct
import subprocess
import sys
import sysconfig
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
SRC = REPO / "mac" / "launcher" / "launcher.c"
CC = shutil.which("cc") or shutil.which("clang") or shutil.which("gcc")

REPORT = '''import json, os, sys
json.dump({"argv": sys.argv[1:], "prefix": sys.prefix, "executable": sys.executable, "cwd": os.getcwd(),
           "home": os.environ.get("CLAUDEOS_HOME"), "pythonpath": os.environ.get("PYTHONPATH"),
           "pythonhome": os.environ.get("PYTHONHOME")},
          open(os.path.join(os.environ["CLAUDEOS_HOME"], "report.json"), "w"))
print("the stand-in ClaudeOS ran")
sys.exit(7)
'''


def shared_libpython():
    name = sysconfig.get_config_var("INSTSONAME") or sysconfig.get_config_var("LDLIBRARY") or ""
    for d in (os.path.join(sys.base_prefix, "lib"), sysconfig.get_config_var("LIBDIR") or ""):
        if name and os.path.isfile(os.path.join(d, name)):
            return os.path.join(d, name)
    return None


LIB = shared_libpython()


@pytest.fixture
def mac_app(tmp_path):
    if not CC or not LIB:
        pytest.skip("needs a C compiler and a shared Python library")
    contents = tmp_path / "home/Applications/ClaudeOS.app/Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    exe = contents / "MacOS/ClaudeOS"
    # Without the alert window on a Mac, which would wait for a click.
    subprocess.run([CC, "-O2", "-Wall", "-Wextra", "-Werror", "-DCLAUDEOS_NO_ALERT", "-o", str(exe), str(SRC), "-ldl"],
                   check=True)
    co = tmp_path / "home/ClaudeOS"
    runtime = co / "runtime"
    subprocess.run([sys.executable, "-m", "venv", "--without-pip", str(runtime)], check=True)
    fake = tmp_path / "fakeapp/claudeos"
    fake.mkdir(parents=True)
    (fake / "__init__.py").write_text("")
    (fake / "__main__.py").write_text(REPORT)
    site = subprocess.run([str(runtime / "bin/python"), "-c", "import sysconfig; print(sysconfig.get_paths()['purelib'])"],
                          capture_output=True, text=True, check=True).stdout.strip()
    Path(site, "claudeos-app.pth").write_text(str(tmp_path / "fakeapp") + "\n")
    conf = co / "launch.conf"
    conf.write_text(f"python={runtime}/bin/python\nlibpython={LIB}\n")
    (tmp_path / "home/json.py").write_text("raise SystemExit('a json.py in the home folder took over')\n")
    env = {"HOME": str(tmp_path / "home"), "PATH": os.environ["PATH"], "PYTHONPATH": "/somewhere/else",
           "PYTHONHOME": "/not/this/python"}
    return exe, co, runtime, conf, env


def test_runs_claudeos_in_its_own_python_environment(mac_app):
    exe, co, runtime, conf, env = mac_app
    r = subprocess.run([str(exe), "-psn_0_12345"], env=env, capture_output=True, text=True, timeout=60)
    assert r.returncode == 7, r.stderr                 # ClaudeOS's exit code is the app's
    rep = json.loads((co / "report.json").read_text())
    assert rep["argv"] == ["run"]                      # the -psn argument macOS may add is ignored
    assert rep["prefix"] == str(runtime) and rep["executable"] == f"{runtime}/bin/python"
    assert rep["cwd"] == env["HOME"] and rep["home"] == str(co)   # and ~/json.py didn't take over (-P)
    assert rep["pythonpath"] is None and rep["pythonhome"] is None    # someone else's Python settings are dropped
    assert "the stand-in ClaudeOS ran" in (co / "logs/app.log").read_text()


def test_not_set_up_says_so(mac_app):
    exe, co, runtime, conf, env = mac_app
    conf.unlink()
    r = subprocess.run([str(exe)], env=env, capture_output=True, text=True, timeout=60)
    assert r.returncode == 1 and "isn't set up on this Mac. Run ClaudeOS Setup" in r.stderr


def test_missing_python_says_so_in_the_log(mac_app):
    exe, co, runtime, conf, env = mac_app
    shutil.rmtree(runtime)
    r = subprocess.run([str(exe)], env=env, capture_output=True, text=True, timeout=60)
    assert r.returncode == 1
    assert "ClaudeOS's Python is missing" in (co / "logs/app.log").read_text()


def _macho(data):
    """(cpu type, whether it has a code signature, the dylibs it uses) of a 64-bit Mach-O program."""
    magic, cputype, _, filetype, ncmds, _, _, _ = struct.unpack_from("<IiiIIIII", data, 0)
    assert magic == 0xFEEDFACF and filetype == 2                 # MH_MAGIC_64, MH_EXECUTE
    off, signed, dylibs = 32, False, []
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, off)
        if cmd == 0x1D:                                          # LC_CODE_SIGNATURE
            signed = True
        if cmd == 0xC:                                           # LC_LOAD_DYLIB
            name_off = struct.unpack_from("<I", data, off + 8)[0]
            dylibs.append(data[off + name_off:off + size].split(b"\0")[0].decode())
        off += size
    return cputype, signed, dylibs


def test_the_mac_app_is_one_signed_program_for_both_kinds_of_mac():
    data = (REPO / "mac/ClaudeOS.app/Contents/MacOS/ClaudeOS").read_bytes()
    magic, count = struct.unpack_from(">II", data, 0)
    assert magic == 0xCAFEBABE and count == 2                      # a universal program
    slices = {}
    for i in range(count):
        cputype, _, offset, size, _ = struct.unpack_from(">iIIII", data, 8 + 20 * i)
        slices[cputype] = _macho(data[offset:offset + size])
    arm, intel = slices[0x0100000C], slices[0x01000007]            # CPU_TYPE_ARM64, CPU_TYPE_X86_64
    assert arm[1] and intel[1]                                     # both signed
    assert arm[2] == intel[2] == ["/usr/lib/libSystem.B.dylib"]
    info = plistlib.loads((REPO / "mac/ClaudeOS.app/Contents/Info.plist").read_bytes())
    assert info["CFBundleIdentifier"] == "com.claudeos.app" and info["CFBundleExecutable"] == "ClaudeOS"
    assert (REPO / "mac/ClaudeOS.app/Contents/_CodeSignature/CodeResources").exists()
    assert (REPO / "mac/ClaudeOS.app/Contents/Info.plist").read_bytes() == (REPO / "mac/app/Info.plist").read_bytes()
