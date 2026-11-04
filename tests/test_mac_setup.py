"""Chat Computer Setup for Mac (mac/setup.sh) on a pretend Mac: macOS's commands are stand-ins that record what they
were asked to do, the disk is tests/mac_pretend.py's, the Asahi installer is tests/mac_fake_installer.py (asking
the real one's questions, answered by the real mac/drive-installer.exp), and the answers are typed in advance.
Also: Chat Computer's first start on the Mac (mac/esp/firstboot.sh) on a pretend Fedora, and the pinned values."""
import base64
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
MAC = REPO / "mac"
TESTS = REPO / "tests"
GB = 1000 ** 3
# The shell setup runs in: macOS's is bash 3.2 (CHATCOMPUTER_TEST_BASH can point at one).
SHELL = os.environ.get("CHATCOMPUTER_TEST_BASH") or "bash"

needs_tools = pytest.mark.skipif(not all(shutil.which(t) for t in ("expect", "rsync", "shasum")),
                                 reason="needs expect, rsync and shasum")
sys.path.insert(0, str(TESTS))
import mac_pretend  # noqa: E402


def _tool(bin_dir: Path, name: str, body: str) -> None:
    f = bin_dir / name
    f.write_text("#!/bin/bash\n" + body + "\n")
    f.chmod(0o755)


def _sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


class PretendMac:
    def __init__(self, tmp_path, chip="Apple M2 Pro", version="15.6", vm="0", admin=True, free=300 * GB,
                 icon_python=True):
        self.tmp = tmp_path
        self.icon_python = icon_python
        self.home = tmp_path / "home"
        self.bin = tmp_path / "bin"
        self.calls = tmp_path / "calls.log"
        self.volumes = tmp_path / "Volumes"
        self.state = tmp_path / "disk.json"
        self.answers_log = tmp_path / "installer-answers.log"
        for d in (self.home, self.bin, self.volumes):
            d.mkdir()
        # The person's folders.
        (self.home / "Desktop").mkdir()
        (self.home / "Desktop/todo.txt").write_text("buy milk\n")
        (self.home / "Desktop/.DS_Store").write_bytes(b"\0finder")
        (self.home / "Documents").mkdir()
        (self.home / "Documents/report.docx").write_bytes(b"PK" + b"r" * 5000)
        (self.home / "Pictures/Photos Library.photoslibrary/originals").mkdir(parents=True)
        (self.home / "Pictures/Photos Library.photoslibrary/originals/IMG_1.heic").write_bytes(b"h" * 3000)
        (self.home / "Pictures/beach.jpg").write_bytes(b"\xff\xd8" + b"j" * 8000)
        mac_pretend.new_mac(self.state, self.volumes, free=free)
        rec = f'echo "$(basename "$0") $*" >> "{self.calls}"'
        _tool(self.bin, "uname", "echo Darwin")
        _tool(self.bin, "sysctl", f'''case "$2" in
  hw.optional.arm64) echo 1 ;;
  machdep.cpu.brand_string) echo "{chip}" ;;
  kern.hv_vmm_present) echo {vm} ;;
esac''')
        _tool(self.bin, "sw_vers", f"echo {version}")
        groups = "staff admin" if admin else "staff"
        _tool(self.bin, "id", f'''case "$1" in
  -Gn) echo "{groups}" ;;
  -F) echo "Sam Lee" ;;
  -un) echo "sammy" ;;
  *) exec /usr/bin/id "$@" ;;
esac''')
        for tool in ("diskutil", "plutil"):
            _tool(self.bin, tool, (rec if tool == "diskutil" else "") + f'\nexec "{sys.executable}" "{TESTS}/mac_pretend.py" {tool} "$@"')
        _tool(self.bin, "du", '[ "$1" = -A ] && exec /usr/bin/du --apparent-size -sk "$3"; exec /usr/bin/du "$@"')
        self.fixtures = tmp_path / "downloads"
        self.fixtures.mkdir()
        _tool(self.bin, "curl", rec + f'''
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift ;; -I|-sSfI) exit 0 ;; http*) url="$1" ;; esac
  shift
done
[ -n "$out" ] || exit 0
cp "{self.fixtures}/$(basename "$url")" "$out"''')
        _tool(self.bin, "networksetup", '''case "$1" in
  -listallhardwareports) printf 'Hardware Port: Wi-Fi\\nDevice: en0\\nEthernet Address: aa\\n' ;;
  -getairportnetwork) echo "Current Wi-Fi Network: Home 5G" ;;
esac''')
        _tool(self.bin, "ipconfig", "exit 0")
        _tool(self.bin, "security", rec + '\necho "from the keychain"')
        for name in ("mdutil", "shutdown", "open"):
            _tool(self.bin, name, rec)
        _tool(self.bin, "bless", rec + '\nread -r pw; echo "bless password: $pw" >> "' + str(self.calls) + '"')
        _tool(self.bin, "sudo", rec + '\n[ "$1" = -v ] && exit 0\nexec "$@"')
        self._downloads()
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", CHATCOMPUTER_MAC_ROOT=str(self.home),
                        CHATCOMPUTER_SETUP_WORK=str(self.home / ".chatcomputer-setup"),
                        CHATCOMPUTER_MANIFEST=str(self.manifest),
                        CHATCOMPUTER_INSTALLER_CMD=f"{sys.executable} {TESTS}/mac_fake_installer.py --pretend-mac",
                        PRETEND_MAC_STATE=str(self.state), FAKE_INSTALLER_LOG=str(self.answers_log))

    def _downloads(self):
        """The Asahi installer (with the Python it brings, which setup uses to add Chat Computer's icon) and a small
        stand-in for Fedora's image, with a manifest that pins them."""
        inst = self.tmp / "inst"
        py = inst / "Frameworks/Python.framework/Versions/3.13/bin/python3.13"
        py.parent.mkdir(parents=True)
        py.write_text(f'#!/bin/sh\nexec "{sys.executable}" "$@"\n' if self.icon_python else "#!/bin/sh\nexit 1\n")
        py.chmod(0o755)
        (inst / "install.sh").write_text("#!/bin/sh\necho the real one is replaced in tests\n")
        tgz = self.fixtures / "installer-v0.9.2.tar.gz"
        with tarfile.open(tgz, "w:gz") as t:
            t.add(inst, arcname=".")
        pkg = self.fixtures / "fedora-44-minimal-202609041600.zip"
        with zipfile.ZipFile(pkg, "w") as z:
            z.writestr("boot.img", b"\0" * 4096)
            z.writestr("esp/EFI/fedora/grub.cfg", "search --fs-uuid --set=root x\n")
        text = (MAC / "manifest.sh").read_text()
        text = re.sub(r'ASAHI_INSTALLER_SHA256="[0-9a-f]+"', f'ASAHI_INSTALLER_SHA256="{_sha(tgz)}"', text)
        text = re.sub(r'FEDORA_PACKAGE_SHA256="[0-9a-f]+"', f'FEDORA_PACKAGE_SHA256="{_sha(pkg)}"', text)
        self.manifest = self.tmp / "manifest.sh"
        self.manifest.write_text(text)

    def run(self, answers, **env):
        e = dict(self.env, **env)
        return subprocess.run([SHELL, str(MAC / "setup.sh")], input="\n".join(answers) + "\n", env=e,
                              capture_output=True, text=True, timeout=300)

    def disk(self):
        return json.loads(self.state.read_text())

    def log(self):
        return self.calls.read_text() if self.calls.exists() else ""


def _env_values(text):
    return {k: base64.b64decode(v).decode() for k, v in (line.split("=", 1) for line in text.splitlines())}


# ------------------------------------------------------------------ installing
@needs_tools
def test_install_on_a_mac(tmp_path):
    mac = PretendMac(tmp_path)
    # Install; bring all folders; Wi-Fi name (Enter: the one in use) and password; go ahead; the Mac's password for
    # the installer; shut down now. Nothing to sign in to: Chat Computer thinks with a model on the Mac itself.
    r = mac.run(["1", "y", "", "correct horse", "y", "hunter2", "y"])
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-2000:]
    answers = [json.loads(line) for line in mac.answers_log.read_text().splitlines()]
    asked = {a["q"]: a["a"] for a in answers}
    # The installer made room (macOS keeps what it uses plus 38 GB, and more), then installed into it.
    assert [a["q"] for a in answers] == ["action", "new size", "continue", "action", "os", "os size", "name", "password"]
    assert asked["name"] == "Chat Computer" and asked["password"] == "hunter2" and asked["os"] == "1"
    new_macos = int(asked["new size"][:-1])
    os_size = int(asked["os size"][:-1])
    assert os_size == 60 * GB                                   # Chat Computer's own space
    disk = mac.disk()
    parts = [p for p in disk["parts"] if not p.get("free")]
    kinds = [(p["content"], p.get("name")) for p in parts]
    assert kinds == [("Apple_APFS_ISC", None), ("Apple_APFS", None), ("Apple_APFS", None), ("EFI", "EFI - CHAT"),
                     ("Linux Filesystem", None), ("Linux Filesystem", None), ("Microsoft Basic Data", "ChatComp"),
                     ("Apple_APFS_Recovery", None)]
    assert parts[1]["size"] == new_macos
    used = 494 * GB - 300 * GB
    assert new_macos >= used + 38 * GB                          # macOS keeps 38 GB free for its updates
    drive = parts[6]
    assert drive["size"] >= 10 * GB                             # what's copied, plus room
    # The folders, copied onto the Chat Computer drive without Finder's files or the Photos library package.
    d = mac.volumes / "ChatComp"
    assert (d / "Desktop/todo.txt").read_text() == "buy milk\n" and not (d / "Desktop/.DS_Store").exists()
    assert (d / "Documents/report.docx").exists() and (d / "Pictures/beach.jpg").exists()
    assert not (d / "Pictures/Photos Library.photoslibrary").exists()
    assert (d / ".metadata_never_index").exists() and (d / ".chatcomputer").is_dir()
    # Chat Computer's first start, on its EFI partition.
    esp = mac.volumes / "EFI - CHAT"
    p = esp / "chatcomputer"
    assert (p / "firstboot.sh").read_text() == (MAC / "esp/firstboot.sh").read_text()
    assert (p / "fedora-grub.cfg").read_text().startswith("search --fs-uuid --set=root 65e9d770")
    grub = (esp / "EFI/fedora/grub.cfg").read_text()
    assert "@" not in grub and "vmlinuz-7.1.6-400.asahi.fc44.aarch64+16k" in grub
    assert "root=UUID=9db8788b-4718-404e-a6d0-1c9694d398fd" in grub and "/boot/efi/chatcomputer/firstboot.sh" in grub
    v = _env_values((p / "setup.env").read_text())
    assert v["USER_NAME"] == "sammy" and v["REAL_NAME"] == "Sam Lee"
    assert v["WIFI_SSID"] == "Home 5G" and v["WIFI_PSK"] == "correct horse"
    assert v["MAC_FILES_PARTUUID"] == drive["uuid"] and re.fullmatch(r"[0-9a-f]{24}", v["INSTALL_ID"])
    assert re.fullmatch(r"[a-km-z2-9]{4}(-[a-km-z2-9]{4}){3}", v["RECOVERY_PASSWORD"])
    assert list((p / "secrets").iterdir()) == []
    assert json.loads((p / "settings.json").read_text())["user_name"] == "Sam Lee"
    with tarfile.open(p / "app.tar.gz") as t:
        names = t.getnames()
    assert "chatcomputer/__init__.py" in names and "linux/install-chatcomputer.sh" in names
    assert not any(n.startswith(("mac/", "windows/")) or "__pycache__" in n for n in names)
    # The recovery password, for repairs, where the person finds it; nothing secret left in setup's folder.
    note = (mac.home / "Documents/Chat Computer recovery password.txt").read_text()
    assert v["RECOVERY_PASSWORD"] in note and "Account: sammy" in note
    work = mac.home / ".chatcomputer-setup"
    assert not (work / "os").exists() and not (work / "payload").exists()
    log = (work / "setup.log").read_text()
    for secret in ("correct horse", "hunter2", v["RECOVERY_PASSWORD"]):
        assert secret not in log and secret not in mac.log()
    calls = mac.log()
    assert "mdutil -i off" in calls and "shutdown -h now" in calls
    assert "Take a photo of these instructions" in r.stdout


@needs_tools
def test_plan_only_changes_nothing(tmp_path):
    mac = PretendMac(tmp_path, vm="1")       # GitHub's Macs are virtual: plan only works there
    r = mac.run(["1", "y"], CHATCOMPUTER_PLAN_ONLY="1")
    assert r.returncode == 0, r.stdout + r.stderr
    assert "Nothing was changed" in r.stdout
    plan = [line for line in (mac.home / ".chatcomputer-setup/setup.log").read_text().splitlines() if line.startswith("PLAN ")]
    assert plan and "store=disk0s2" in plan[0] and "chatcomputer=60000000000" in plan[0]
    assert mac.disk()["parts"] == mac_pretend.new_mac(tmp_path / "again.json", mac.volumes)["parts"]
    assert "addPartition" not in mac.log()


CRASHING_INSTALLER = f"{sys.executable} {TESTS}/mac_fake_installer.py --pretend-mac --crash"
ANSWERS_TO_THE_INSTALLER = ["1", "y", "", "correct horse", "y"]


@needs_tools
def test_running_setup_again_after_a_stop_doesnt_download_fedora_again(tmp_path):
    mac = PretendMac(tmp_path)
    r = mac.run(ANSWERS_TO_THE_INSTALLER, CHATCOMPUTER_INSTALLER_CMD=CRASHING_INSTALLER)
    assert r.returncode == 1 and "The Asahi installer didn't finish" in r.stdout, r.stdout[-2000:]
    pkg = mac.home / ".chatcomputer-setup/os/fedora-44-minimal-202609041600.zip"
    with zipfile.ZipFile(pkg) as z:
        assert "chatcomputer.icns" in z.namelist()              # Chat Computer's icon went in
    assert (pkg.parent / (pkg.name + ".with-icon")).read_text().strip() == _sha(pkg)
    r = mac.run(ANSWERS_TO_THE_INSTALLER + ["hunter2", "y"])
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-2000:]
    assert "downloaded already" in r.stdout
    downloads = [c for c in mac.log().splitlines() if c.startswith("curl ") and c.endswith(".zip")]
    assert len(downloads) == 1, downloads                       # downloaded once


@needs_tools
def test_without_the_icon_the_installer_isnt_asked_for_it(tmp_path):
    mac = PretendMac(tmp_path, icon_python=False)
    r = mac.run(ANSWERS_TO_THE_INSTALLER, CHATCOMPUTER_INSTALLER_CMD=CRASHING_INSTALLER)
    assert "icon couldn't be added" in r.stdout
    work = mac.home / ".chatcomputer-setup"
    data = json.loads((work / "asahi-install/installer_data.json").read_text())
    assert "icon" not in data["os_list"][0] and data["os_list"][0]["package"].endswith(".zip")
    pkg = work / "os/fedora-44-minimal-202609041600.zip"
    assert _sha(pkg) == _sha(mac.fixtures / pkg.name)          # the package as downloaded
    assert not (work / "os" / (pkg.name + ".with-icon")).exists()


@pytest.mark.parametrize("chip, words", [
    ("Apple M4 Pro", "M1, M2 or M3"),
    ("Apple M5", "M1, M2 or M3"),
    ("Apple M3 Ultra", "M3 Ultra"),
])
def test_macs_chatcomputer_cant_start_on_are_turned_away(tmp_path, chip, words):
    mac = PretendMac(tmp_path, chip=chip)
    r = mac.run(["1"])
    assert r.returncode == 1 and words in r.stdout
    assert "addPartition" not in mac.log()


def test_old_macos_and_non_admins_are_turned_away(tmp_path):
    mac = PretendMac(tmp_path, version="13.4")
    r = mac.run(["1"])
    assert r.returncode == 1 and "macOS 13.5 or newer" in r.stdout
    other = tmp_path / "other"
    other.mkdir()
    r = PretendMac(other, admin=False).run(["1"])
    assert r.returncode == 1 and "administrator" in r.stdout


def test_a_virtual_mac_is_not_installed_on(tmp_path):
    mac = PretendMac(tmp_path, vm="1")
    r = mac.run(["1"])
    assert r.returncode == 1 and "virtual Mac" in r.stdout


@needs_tools
def test_too_little_space_stops_before_anything_changes(tmp_path):
    mac = PretendMac(tmp_path, free=60 * GB)       # 38 GB must stay free: 22 GB left, less than Chat Computer needs
    r = mac.run(["1", "y"])
    assert r.returncode == 1 and "Free up space in macOS" in r.stdout
    assert "addPartition" not in mac.log()


@needs_tools
def test_already_installed_is_noticed(tmp_path):
    mac = PretendMac(tmp_path)
    st = mac.disk()
    mac_pretend.resize_macos(st, 400 * GB)
    mac_pretend.install_os(st, 60 * GB)
    mac.state.write_text(json.dumps(st))
    r = mac.run(["1"])
    assert r.returncode == 1 and "already on this Mac" in r.stdout


# ------------------------------------------------------------------ after installing
def _installed(mac):
    st = mac.disk()
    mac_pretend.resize_macos(st, 400 * GB)
    made = mac_pretend.install_os(st, 60 * GB)
    i = [p.get("id") for p in st["parts"]].index(made["Root"]) + 1
    st["parts"][i] = {"id": "disk0s9", "content": "Microsoft Basic Data", "size": st["parts"][i]["size"],
                      "name": "ChatComp", "fs": True, "mounted": True, "uuid": "AB12-CD34"}
    mac.state.write_text(json.dumps(st))
    d = mac_pretend.folder(st, "ChatComp")
    Path(d, "Documents").mkdir()
    Path(d, "Documents/notes from Chat Computer.txt").write_text("hello from Chat Computer\n")
    return made


@needs_tools
def test_removing_chatcomputer_gives_macos_everything_back(tmp_path):
    mac = PretendMac(tmp_path)
    made = _installed(mac)
    # Remove; copy the drive's files to macOS first; REMOVE; the Mac's password for the start-up disk.
    r = mac.run(["4", "y", "REMOVE", "hunter2"])
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-2000:]
    calls = mac.log()
    # macOS is the start-up disk again before anything is deleted, then Chat Computer's pieces go, then macOS grows.
    order = [calls.index(s) for s in ("bless --setBoot", f"diskutil apfs deleteContainer {made['Stub']}",
                                      f"diskutil eraseVolume free free {made['EFI']}",
                                      "diskutil eraseVolume free free disk0s9", "diskutil apfs resizeContainer disk0s2 0")]
    assert order == sorted(order)
    assert "bless --setBoot --device /dev/disk3s1 " in calls     # macOS's system volume, not the snapshot / is on
    assert "bless password: hunter2" in calls
    parts = [p for p in mac.disk()["parts"] if not p.get("free")]
    assert [p["content"] for p in parts] == ["Apple_APFS_ISC", "Apple_APFS", "Apple_APFS_Recovery"]
    assert parts[1]["size"] == 494 * GB                     # macOS has all its space back
    kept = mac.home / "Chat Computer files/Documents/notes from Chat Computer.txt"
    assert kept.read_text() == "hello from Chat Computer\n"
    assert "hunter2" not in (mac.home / ".chatcomputer-setup/setup.log").read_text()


@needs_tools
def test_space_left_after_a_removal_goes_back_to_macos_on_the_next_run(tmp_path):
    mac = PretendMac(tmp_path)
    _installed(mac)
    st = mac.disk()
    st["resize_fails"] = 1                                   # macOS's partition can't grow the first time
    mac.state.write_text(json.dumps(st))
    r = mac.run(["4", "n", "REMOVE", "hunter2"])
    assert r.returncode == 0 and "couldn't grow" in r.stdout, r.stdout[-2000:]
    assert mac.disk()["parts"][1]["size"] == 400 * GB
    r = mac.run(["4", "y"])                                  # again: Chat Computer is gone, the space comes back
    assert r.returncode == 0 and "free space is right after macOS's partition" in r.stdout, r.stdout[-2000:]
    assert mac.disk()["parts"][1]["size"] == 494 * GB
    r = mac.run(["4"])
    assert r.returncode == 0 and "isn't installed on this Mac. Nothing was changed." in r.stdout


@needs_tools
def test_removing_needs_the_typed_word(tmp_path):
    mac = PretendMac(tmp_path)
    _installed(mac)
    r = mac.run(["4", "n", "keep it"])
    assert r.returncode == 0 and "Nothing was changed" in r.stdout
    assert "deleteContainer" not in mac.log() and "eraseVolume" not in mac.log()


@needs_tools
def test_sending_more_files(tmp_path):
    mac = PretendMac(tmp_path)
    _installed(mac)
    (mac.home / "Music").mkdir()
    (mac.home / "Music/song.m4a").write_bytes(b"m" * 2000)
    # Send files; not all; then per folder: Desktop no, Documents no, Pictures no, Music yes.
    r = mac.run(["2", "n", "n", "n", "n", "y"])
    assert r.returncode == 0, r.stdout + r.stderr
    d = mac.volumes / "ChatComp"
    assert (d / "Music/song.m4a").exists() and not (d / "Desktop").exists()


@needs_tools
def test_choosing_what_starts(tmp_path):
    mac = PretendMac(tmp_path)
    _installed(mac)
    r = mac.run(["3", "1", "hunter2"])
    assert r.returncode == 0, r.stdout + r.stderr
    assert "bless --setBoot --device /dev/disk5s1" in mac.log()        # Chat Computer's start-up volume


# ------------------------------------------------------------------ the installer's questions
def _drive(tmp_path, *args, macos="374000000000B", os_size="60000000000B", typed="hunter2\n"):
    log = tmp_path / "answers.log"
    env = dict(os.environ, FAKE_INSTALLER_LOG=str(log), CHATCOMPUTER_MACOS_SIZE=macos, CHATCOMPUTER_OS_SIZE=os_size)
    r = subprocess.run(["expect", "-f", str(MAC / "drive-installer.exp"), "--", sys.executable,
                        str(TESTS / "mac_fake_installer.py"), *args], input=typed, env=env, capture_output=True,
                       text=True, timeout=120)
    answers = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    return r, answers


@needs_tools
def test_installer_questions_answered_with_free_space_already_there(tmp_path):
    r, answers = _drive(tmp_path, "--no-resize", macos="")
    assert r.returncode == 0, r.stdout
    assert [a["q"] for a in answers] == ["action", "os", "os size", "name", "password"]
    assert answers[0]["a"] == "f"


@needs_tools
def test_installer_asks_the_password_again_after_a_typo(tmp_path):
    r, answers = _drive(tmp_path, "--bless-fails-once", typed="wrong\nhunter2\n")
    assert r.returncode == 0, r.stdout
    assert [a["a"] for a in answers if a["q"] == "password"] == ["wrong", "hunter2"]


@needs_tools
@pytest.mark.parametrize("flag, code", [("--two-areas", 4), ("--small", 3), ("--crash", 6)])
def test_installer_questions_setup_cant_answer_stop_it(tmp_path, flag, code):
    r, answers = _drive(tmp_path, flag)
    assert r.returncode == code, r.stdout
    assert "Chat Computer Setup:" in r.stdout
    assert not any(a["q"] == "password" for a in answers)


# ------------------------------------------------------------------ the first start, on the Mac
def _first_start(tmp_path, setup_env=None, app=True):
    root = tmp_path / "root"
    esp = tmp_path / "esp"
    bin_dir = tmp_path / "bin"
    calls = tmp_path / "calls.log"
    for d in ("etc/systemd/system/multi-user.target.wants", "usr/share/zoneinfo/Asia", "home"):
        (root / d).mkdir(parents=True, exist_ok=True)
    (root / "usr/share/zoneinfo/Asia/Singapore").write_text("TZif")
    (root / "etc/reconfigSys").write_text("")
    (root / "etc/systemd/system/multi-user.target.wants/initial-setup.service").symlink_to(
        "/usr/lib/systemd/system/initial-setup.service")
    p = esp / "chatcomputer"
    (p / "secrets").mkdir(parents=True)
    (esp / "EFI/fedora").mkdir(parents=True)
    (esp / "EFI/fedora/grub.cfg").write_text("Chat Computer first start\n")
    (p / "fedora-grub.cfg").write_text("Fedora's own\n")
    (p / "settings.json").write_text('{"user_name": "Sam Lee"}\n')
    values = {"USER_NAME": "sammy", "REAL_NAME": "Sam Lee", "RECOVERY_PASSWORD": "abcd-efgh-jkmn-pqrs",
              "TIMEZONE": "Asia/Singapore", "INSTALL_ID": "0123456789abcdef01234567",
              "MAC_FILES_PARTUUID": "4B1E2F3A-0000-4000-8000-00000000C0DE", "WIFI_SSID": 'Home "5G"\\',
              "WIFI_PSK": "correct horse"}
    values.update(setup_env or {})
    (p / "setup.env").write_text("".join(f"{k}={base64.b64encode(v.encode()).decode()}\n" for k, v in values.items()))
    if app:
        src = tmp_path / "app"
        (src / "chatcomputer").mkdir(parents=True)
        (src / "chatcomputer/__init__.py").write_text("")
        (src / "linux/firstboot").mkdir(parents=True)
        shutil.copy(REPO / "linux/firstboot/chatcomputer-finish-install.service", src / "linux/firstboot/")
        with tarfile.open(p / "app.tar.gz", "w:gz") as t:
            for name in ("chatcomputer", "linux"):
                t.add(src / name, arcname=name)
    bin_dir.mkdir()
    rec = f'echo "$(basename "$0") $*" >> "{calls}"'
    users = tmp_path / "users"
    _tool(bin_dir, "useradd", rec + f'\necho "${{@: -1}}" >> "{users}"; mkdir -p "{root}/home/${{@: -1}}"')
    _tool(bin_dir, "id", f'grep -qx "$2" "{users}" 2>/dev/null || exit 1; echo {os.getuid()}')
    _tool(bin_dir, "chpasswd", rec + f'\ncat >> "{tmp_path}/chpasswd.in"')
    _tool(bin_dir, "restorecon", rec)
    _tool(bin_dir, "sleep", "exit 0")
    env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", CHATCOMPUTER_ESP=str(esp), CHATCOMPUTER_TARGET=str(root),
               CHATCOMPUTER_CONSOLE="/dev/null")
    r = subprocess.run(["sh", str(MAC / "esp/firstboot.sh")], env=env, capture_output=True, text=True, timeout=60)
    return r, root, esp, calls


def test_first_start_on_the_mac(tmp_path):
    r, root, esp, calls = _first_start(tmp_path)
    assert r.returncode == 0, r.stdout + r.stderr + (esp / "chatcomputer/firstboot.log").read_text()
    log = calls.read_text()
    assert "useradd --create-home --comment Sam Lee --groups wheel sammy" in log
    assert (tmp_path / "chpasswd.in").read_text() == "sammy:abcd-efgh-jkmn-pqrs\n"
    wifi = root / "etc/NetworkManager/system-connections/chatcomputer-wifi.nmconnection"
    text = wifi.read_text()
    assert 'ssid=Home "5G"\\\\' in text and "psk=correct horse" in text and "key-mgmt=wpa-psk" in text
    assert stat.S_IMODE(wifi.stat().st_mode) == 0o600
    assert (root / "opt/chatcomputer/app/chatcomputer/__init__.py").exists()
    h = root / "home/sammy/ChatComputer"
    assert stat.S_IMODE((h / "secrets").stat().st_mode) == 0o700 and list((h / "secrets").iterdir()) == []
    assert (root / "home/sammy/Mac").is_dir()
    assert (root / "etc/chatcomputer/install-user").read_text() == "sammy\n"
    assert (root / "etc/chatcomputer/mac-files-partuuid").read_text() == "4b1e2f3a-0000-4000-8000-00000000c0de\n"
    unit = root / "etc/systemd/system/multi-user.target.wants/chatcomputer-finish-install.service"
    assert os.readlink(unit) == "/etc/systemd/system/chatcomputer-finish-install.service"
    # Nothing of Fedora's may listen while the finish-install service runs (Chat Computer has no SSH server or cockpit).
    for name in ("sshd.service", "sshd.socket", "cockpit.service", "cockpit.socket"):
        mask = root / "etc/systemd/system" / name
        assert mask.is_symlink(), name
        assert os.readlink(mask) == "/dev/null", name
    assert os.readlink(root / "etc/localtime") == "../usr/share/zoneinfo/Asia/Singapore"
    assert (root / "etc/hostname").read_text() == "chatcomputer\n"
    # Fedora's own first-start questions are off, its usual start is back, and nothing of setup's is left.
    assert not (root / "etc/reconfigSys").exists()
    assert not (root / "etc/systemd/system/multi-user.target.wants/initial-setup.service").is_symlink()
    assert (esp / "EFI/fedora/grub.cfg").read_text() == "Fedora's own\n"
    for gone in ("setup.env", "secrets", "app.tar.gz", "settings.json"):
        assert not (esp / "chatcomputer" / gone).exists(), gone
    assert "restorecon -RF" in log
    assert "correct horse" not in (esp / "chatcomputer/firstboot.log").read_text()


def test_first_start_tries_again_then_falls_back_to_fedora(tmp_path):
    r, root, esp, _ = _first_start(tmp_path, app=False)
    assert r.returncode == 1
    assert (esp / "EFI/fedora/grub.cfg").read_text() == "Chat Computer first start\n"  # tries again next start
    assert (esp / "chatcomputer/tries").read_text().strip() == "1"
    for name in ("sshd.service", "sshd.socket", "cockpit.service", "cockpit.socket"):
        assert os.readlink(root / "etc/systemd/system" / name) == "/dev/null", name   # masked before anything fails
    assert (root / "etc/reconfigSys").exists()                                       # Fedora's questions still on
    assert (esp / "chatcomputer/setup.env").exists() and (esp / "chatcomputer/settings.json").exists()  # for the retry
    (esp / "chatcomputer/tries").write_text("2\n")
    env = dict(os.environ, PATH=f"{tmp_path / 'bin'}:{os.environ['PATH']}", CHATCOMPUTER_ESP=str(esp),
               CHATCOMPUTER_TARGET=str(root), CHATCOMPUTER_CONSOLE="/dev/null")
    r = subprocess.run(["sh", str(MAC / "esp/firstboot.sh")], env=env, capture_output=True, text=True, timeout=60)
    assert r.returncode == 1
    assert (esp / "EFI/fedora/grub.cfg").read_text() == "Fedora's own\n"            # third time: Fedora's start
    for gone in ("setup.env", "secrets", "settings.json"):                          # and no sign-in left behind
        assert not (esp / "chatcomputer" / gone).exists(), gone


def test_first_start_refuses_a_bad_account_name(tmp_path):
    r, root, esp, calls = _first_start(tmp_path, {"USER_NAME": "Sam Lee"})
    assert r.returncode == 1 and not calls.exists()
    assert "account name" in (esp / "chatcomputer/firstboot.log").read_text()


# ------------------------------------------------------------------ the pinned values agree
def _manifest():
    out = {}
    for line in (MAC / "manifest.sh").read_text().splitlines():
        m = re.match(r'^([A-Z0-9_]+)="?([^"]*)"?$', line)
        if m:
            out[m.group(1)] = m.group(2)
    return out


def test_manifest_installer_data_and_first_start_agree():
    m = _manifest()
    data = json.loads((MAC / "installer_data.json").read_text())
    (entry,) = data["os_list"]
    assert entry["name"] == entry["default_os_name"] == "Chat Computer"
    assert entry["package"] == m["FEDORA_PACKAGE"] == m["FEDORA_PACKAGE_URL"].rsplit("/", 1)[1]
    assert "14.8.3" in entry["supported_fw"]                     # M3 Macs need it
    # Fedora's own partitions for that build (fedora-44-minimal-202609041600.json), with its ESP serial: the
    # image's /etc/fstab mounts the EFI partition as UUID=1318-2C36.
    assert entry["partitions"] == [
        {"name": "EFI", "type": "EFI", "size": "524288000B", "format": "fat", "volume_id": "0x13182c36",
         "copy_firmware": True, "copy_installer_data": True, "source": "esp"},
        {"name": "Boot", "type": "Linux", "size": "1073741824B", "image": "boot.img"},
        {"name": "Root", "type": "Linux", "size": "4216303616B", "expand": True, "image": "root.img"}]
    assert entry["icon"] == "chatcomputer.icns" and (MAC / "chatcomputer.icns").read_bytes()[:4] == b"icns"
    for k in ("ASAHI_INSTALLER_SHA256", "FEDORA_PACKAGE_SHA256"):
        assert re.fullmatch(r"[0-9a-f]{64}", m[k]), k
    assert m["ASAHI_INSTALLER_URL"].endswith(f"installer-{m['ASAHI_INSTALLER_VERSION']}.tar.gz")
    grub = (MAC / "esp/grub-firststart.cfg").read_text()
    assert {"@KERNEL@", "@BOOT_UUID@", "@ROOT_UUID@"} <= set(re.findall(r"@[A-Z_]+@", grub))


def _grub_words(line):
    """GRUB's script lexer, for what a linux line uses: words split on blanks, '...' and "..." quoting removed."""
    words, cur, quote, have = [], "", None, False
    for ch in line:
        if quote:
            if ch == quote:
                quote = None
            else:
                cur += ch
        elif ch in "'\"":
            quote, have = ch, True
        elif ch in " \t":
            if cur or have:
                words.append(cur)
            cur, have = "", False
        else:
            cur += ch
    if cur or have:
        words.append(cur)
    return words


def _grub_cmdline(args):
    """grub_create_loader_cmdline: \\, ' and " escaped with a backslash; an argument with a space in quotes."""
    out = []
    for a in args:
        e = "".join("\\" + c if c in "\\'\"" else c for c in a)
        out.append(f'"{e}"' if " " in a else e)
    return " ".join(out)


def _systemd_cmdline(line):
    """systemd's reading of /proc/cmdline: extract_first_word with UNQUOTE|RELAX|RETAIN_ESCAPE (backslash is
    an ordinary character there)."""
    words, cur, quote, have = [], "", None, False
    for ch in line:
        if quote:
            if ch == quote:
                quote = None
            else:
                cur += ch
        elif ch in "'\"":
            quote, have = ch, True
        elif ch.isspace():
            if cur or have:
                words.append(cur)
            cur, have = "", False
        else:
            cur += ch
    if cur or have:
        words.append(cur)
    return words


def test_first_start_command_reaches_systemd_whole():
    # What happened in an emulated Mac: a systemd.run= value with quotes inside reached systemd broken up.
    line = next(l for l in (MAC / "esp/grub-firststart.cfg").read_text().splitlines() if l.startswith("linux "))
    kernel_cmdline = _grub_cmdline(_grub_words(line)[2:])
    words = _systemd_cmdline(kernel_cmdline)
    assert "systemd.run=/bin/sh /boot/efi/chatcomputer/firstboot.sh" in words, words
    assert {"systemd.run_success_action=reboot", "systemd.run_failure_action=reboot", "rootflags=subvol=root"} <= set(words)
    # systemd-run-generator writes its default.target where /etc's wins (Fedora's image has one), so the unit
    # to start is named outright.
    assert "systemd.unit=kernel-command-line.target" in words
    # The old way, for the record: GRUB escapes the inner quotes, and systemd keeps the backslashes.
    old = _systemd_cmdline(_grub_cmdline(_grub_words("""linux x 'systemd.run="/bin/sh /boot/efi/chatcomputer/firstboot.sh"'""")[2:]))
    assert "systemd.run=/bin/sh /boot/efi/chatcomputer/firstboot.sh" not in old


@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="needs shellcheck")
def test_mac_scripts_shellcheck():
    r = subprocess.run(["shellcheck", "-s", "bash", str(MAC / "setup.sh")], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout
    r = subprocess.run(["shellcheck", "-s", "sh", str(MAC / "esp/firstboot.sh")], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout
