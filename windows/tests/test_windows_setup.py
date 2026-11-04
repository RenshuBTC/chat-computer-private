"""Checks for Chat Computer Setup (the Windows tool) that can run on Linux.

* The PowerShell unit tests (windows/tests/Run-Tests.ps1) under pwsh.
* The generated installer answers against the Ubuntu installer's own schema.
* The password hashes against Linux's crypt.
* The two scripts that run inside the installer (shellcheck, and finish.sh against a fake system).
* Windows PowerShell 5.1 compatibility (PSScriptAnalyzer) and plain-ASCII sources.

Tool locations can be given with CHATCOMPUTER_PWSH, CHATCOMPUTER_PSSA (folder containing the
PSScriptAnalyzer module) and CHATCOMPUTER_SUBIQUITY_SCHEMA; tests that need a missing tool are skipped.
"""
import json
import os
import re
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

WIN = Path(__file__).resolve().parents[1]
REPO = WIN.parent
SCRATCH = Path(os.environ.get("CHATCOMPUTER_TEST_SCRATCH", str(Path.home() / ".cache" / "chatcomputer-test" / "windows-setup")))


def _find(env, *candidates):
    v = os.environ.get(env)
    if v and Path(v).exists():
        return Path(v)
    for c in candidates:
        if c and Path(c).exists():
            return Path(c)
    return None


PWSH = _find("CHATCOMPUTER_PWSH", shutil.which("pwsh"), SCRATCH / "tools/pwsh/pwsh")
PSSA = _find("CHATCOMPUTER_PSSA", SCRATCH / "tools/modules")
SCHEMA = _find("CHATCOMPUTER_SUBIQUITY_SCHEMA", SCRATCH / "dl/subiquity-noble/autoinstall-schema.json")
needs_pwsh = pytest.mark.skipif(PWSH is None, reason="pwsh not available")


@pytest.fixture(scope="module")
def generated(tmp_path_factory):
    if PWSH is None:
        pytest.skip("pwsh not available")
    out = tmp_path_factory.mktemp("winsetup")
    r = subprocess.run([str(PWSH), "-NoProfile", "-File", str(WIN / "tests/Run-Tests.ps1"), "-OutDir", str(out)],
                       capture_output=True, text=True, timeout=600)
    assert r.returncode == 0, r.stdout + r.stderr
    return out


def _load(path):
    import yaml
    data = yaml.safe_load(Path(path).read_text(encoding="utf-8"))
    assert list(data) == ["autoinstall"]
    return data["autoinstall"]


@needs_pwsh
def test_powershell_unit_tests(generated):
    assert (generated / "autoinstall-wifi.yaml").exists()


@pytest.mark.parametrize("name", ["autoinstall-wifi.yaml", "autoinstall-wired.yaml"])
def test_answers_match_the_installer_schema(generated, name):
    if SCHEMA is None:
        pytest.skip("subiquity autoinstall schema not available")
    import jsonschema
    schema = json.loads(SCHEMA.read_text())
    ai = _load(generated / name)
    jsonschema.validate(ai, schema)


def test_answers_only_use_free_space_and_run_chatcomputer_steps(generated):
    ai = _load(generated / "autoinstall-wifi.yaml")
    # Free space only, on the disk the installer itself is on (setup put it on the disk Chat Computer goes on).
    assert ai["storage"]["layout"] == {"name": "direct", "mode": "use_gap", "match": {"install-media": True}}
    assert ai["early-commands"] == [["sh", "/cdrom/chatcomputer/guard.sh", "0123456789abcdef01234567"],
                                    ["sh", "/cdrom/chatcomputer/network.sh"]]
    assert ai["late-commands"] == [["sh", "/cdrom/chatcomputer/finish.sh", "sammy", "0123456789abcdef01234567"]]
    assert ai["identity"]["username"] == "sammy"
    assert ai["identity"]["realname"] == 'Sam Lee "é"'
    assert ai["kernel"] == {"flavor": "hwe"}
    assert ai["shutdown"] == "reboot"
    ap = ai["network"]["wifis"]["any-wifi"]["access-points"]
    assert ap == {'Home "5G"': {"password": 'p@ss w0rd"\\'}}
    wired = _load(generated / "autoinstall-wired.yaml")
    assert "wifis" not in wired["network"]


def test_password_hashes_match_linux(generated):
    import warnings
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        crypt = pytest.importorskip("crypt")
    vectors = json.loads((generated / "crypt-vectors.json").read_text(encoding="utf-8"))
    assert len(vectors) == 15
    for v in vectors:
        assert crypt.crypt(v["password"], "$6$" + v["salt"]) == v["hash"], v


def test_boot_menu(generated):
    text = (generated / "grub.cfg").read_text()
    assert text.splitlines()[0] == "set timeout=5"
    assert text.splitlines()[1] == ""
    assert text.count(" autoinstall ") == 2
    # Without storage version 2 the installer finds no free space on a disk that has partitions.
    assert text.count(" autoinstall subiquity-storage-version=2 ") == 2
    assert text.index('menuentry "Install Chat Computer" {') < text.index('menuentry "Install Chat Computer (older kernel)" {')
    # Once installed (finish.sh's marker is on the PC), the installer starts Chat Computer instead of installing again.
    marker = "/EFI/chatcomputer/installed-0123456789abcdef01234567"
    assert text.index(f"if search --no-floppy --file --set=chatcomputer_esp {marker}; then") \
        < text.index('menuentry "Start Chat Computer') < text.index("\nelse\n") < text.index('menuentry "Install Chat Computer" {')
    assert "\r" not in text


@pytest.mark.skipif(shutil.which("grub-script-check") is None, reason="grub-script-check not available")
def test_boot_menu_is_valid_grub_script(generated):
    r = subprocess.run(["grub-script-check", str(generated / "grub.cfg")], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr


def test_settings_json(generated):
    data = json.loads((generated / "settings.json").read_text(encoding="utf-8"))
    assert data == {"user_name": 'Sam "R" Lee', "windows_user": "sammy", "timezone": "Asia/Singapore",
                    "locale": "en_US.UTF-8"}


def test_names_and_wifi_with_wide_characters_reach_the_installer_whole(generated):
    # An emoji, or a Chinese character past the first 65,536, used to be escaped as two halves: Python's YAML
    # reader then gave two broken characters (that can't be written out again), and libyaml's refused the file.
    import yaml
    text = (generated / "autoinstall-wide.yaml").read_text(encoding="utf-8")
    loaders = [yaml.SafeLoader] + ([yaml.CSafeLoader] if getattr(yaml, "__with_libyaml__", False) else [])
    for loader in loaders:
        ai = yaml.load(text, Loader=loader)["autoinstall"]
        assert ai["identity"]["realname"] == "Sam \U00020BB7\U0001F600", loader
        assert list(ai["network"]["wifis"]["any-wifi"]["access-points"]) == ["\U0001F3E0 Home"], loader
        assert ai["identity"]["realname"].encode("utf-8")
    data = json.loads((generated / "settings-wide.json").read_text(encoding="utf-8"))
    assert data["user_name"] == "Sam \U00020BB7\U0001F600"


# ------------------------------------------------------------------ scripts in the installer partition
@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not available")
def test_installer_scripts_shellcheck():
    r = subprocess.run(["shellcheck", "-s", "sh", str(WIN / "installer/guard.sh"), str(WIN / "installer/network.sh"),
                        str(WIN / "installer/finish.sh")],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stdout


def _run_finish(tmp_path, installer_works=True, bitlocker_key=None, update_source=None):
    """finish.sh against a fake installer partition and a fake new system; curtin is a stand-in that records its calls."""
    src = tmp_path / "cdrom/chatcomputer"
    target = tmp_path / "target"
    (src / "app/linux/session").mkdir(parents=True)
    (src / "app/linux/firstboot").mkdir(parents=True)
    (src / "app/chatcomputer").mkdir(parents=True)
    (src / "app/linux/install-chatcomputer.sh").write_text("#!/bin/bash\necho install\n")
    for unit in ("chatcomputer-finish-install.service", "chatcomputer-reclaim-installer.service"):
        (src / "app/linux/firstboot" / unit).write_text((REPO / "linux/firstboot" / unit).read_text())
    (src / "app/linux/session/chatcomputer-session").write_text("#!/bin/bash\n")
    (src / "app/linux/session/xinitrc").write_text("exec something\n")
    (src / "app/chatcomputer/__init__.py").write_text("")
    (src / "secrets").mkdir()
    (src / "settings.json").write_text("{}\n")
    if bitlocker_key:
        (src / "secrets/bitlocker-key").write_text(bitlocker_key + "\n")
    if update_source:
        (src / "update-source").write_text(update_source + "\n", encoding="utf-8")
    for p in src.rglob("*"):
        if p.is_file():
            p.chmod(0o777)        # FAT32 shows every file as executable
    (target / "home/sammy").mkdir(parents=True)
    (target / "boot/efi/EFI").mkdir(parents=True)
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    calls = tmp_path / "curtin-calls"
    fail = 'case "$*" in *install-chatcomputer.sh*) exit 1 ;; esac\n' if not installer_works else ""
    fake = bin_dir / "curtin"
    fake.write_text(f'#!/bin/sh\necho "$@" >> "{calls}"\n{fail}exit 0\n')
    fake.chmod(0o755)
    env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", CHATCOMPUTER_FINISH_RETRY_WAIT="0",
               CHATCOMPUTER_FINISH_SRC=str(src), CHATCOMPUTER_FINISH_TARGET=str(target))
    r = subprocess.run(["sh", str(WIN / "installer/finish.sh"), "sammy", "0123456789abcdef"], env=env,
                       capture_output=True, text=True)
    return r, target, calls.read_text().splitlines()


def test_finish_script_against_a_fake_system(tmp_path):
    r, target, lines = _run_finish(tmp_path)
    assert r.returncode == 0, r.stdout + r.stderr
    app = target / "opt/chatcomputer/app"
    assert (app / "linux/install-chatcomputer.sh").exists()
    mode = lambda p: stat.S_IMODE(p.stat().st_mode)  # noqa: E731
    assert mode(app / "linux/session/chatcomputer-session") == 0o755
    assert mode(app / "linux/session/xinitrc") == 0o644
    assert mode(app / "chatcomputer/__init__.py") == 0o644
    assert mode(target / "home/sammy/ChatComputer/secrets") == 0o700
    assert list((target / "home/sammy/ChatComputer/secrets").iterdir()) == []      # nothing to sign in with
    assert mode(target / "home/sammy/ChatComputer/settings.json") == 0o644
    # The installer's boot menu (and guard.sh) look for this file to tell that this installer already installed here.
    assert (target / "boot/efi/EFI/chatcomputer/installed-0123456789abcdef").read_text().splitlines()[0] == "id=0123456789abcdef"
    assert any("chown -R sammy:sammy /home/sammy/ChatComputer" in ln for ln in lines)
    # The first-start safety net is in place (enabled = the wants link)...
    assert (target / "etc/chatcomputer/install-user").read_text() == "sammy\n"
    assert (target / "etc/systemd/system/chatcomputer-finish-install.service").exists()
    link = target / "etc/systemd/system/multi-user.target.wants/chatcomputer-finish-install.service"
    assert os.readlink(link) == "/etc/systemd/system/chatcomputer-finish-install.service"
    # ...and the long step ran once.
    assert sum("install-chatcomputer.sh --user sammy --from-installer" in ln for ln in lines) == 1
    # The first start's clean-up of the installer partition: enabled, and told which installer is this one's.
    assert (target / "etc/chatcomputer/install-id").read_text() == "0123456789abcdef\n"
    assert (target / "etc/systemd/system/chatcomputer-reclaim-installer.service").exists()
    link = target / "etc/systemd/system/multi-user.target.wants/chatcomputer-reclaim-installer.service"
    assert os.readlink(link) == "/etc/systemd/system/chatcomputer-reclaim-installer.service"


def test_finish_script_gives_chatcomputer_its_bitlocker_key(tmp_path):
    r, target, _ = _run_finish(tmp_path, bitlocker_key="111111-222222-333333-444444-555555-666666-777777-888888")
    assert r.returncode == 0, r.stdout + r.stderr
    key = target / "etc/chatcomputer/bitlocker-recovery-key"
    assert key.read_text() == "111111-222222-333333-444444-555555-666666-777777-888888\n"
    assert stat.S_IMODE(key.stat().st_mode) == 0o600 and key.stat().st_uid == 0
    # Without one (Windows' drive isn't encrypted), there's no key file.
    r, target, _ = _run_finish(tmp_path / "plain")
    assert not (target / "etc/chatcomputer/bitlocker-recovery-key").exists()


def test_finish_script_notes_where_updates_come_from(tmp_path):
    # The folder Chat Computer Setup ran from, on Windows' drive: newer versions put there update Chat Computer. Root's file.
    r, target, _ = _run_finish(tmp_path, update_source="Users/Sam Lee/Downloads/ChatComputer/ChatComputer")
    assert r.returncode == 0, r.stdout + r.stderr
    f = target / "etc/chatcomputer/update-source"
    assert f.read_text(encoding="utf-8") == "Users/Sam Lee/Downloads/ChatComputer/ChatComputer\n"
    assert stat.S_IMODE(f.stat().st_mode) == 0o644 and f.stat().st_uid == 0
    # Setup not on Windows' own drive: no updates from there.
    r, target, _ = _run_finish(tmp_path / "elsewhere")
    assert r.returncode == 0 and not (target / "etc/chatcomputer/update-source").exists()


def test_finish_script_names_the_chatcomputer_partition(tmp_path):
    bin_dir = tmp_path / "tools"
    bin_dir.mkdir()
    for name, body in (("findmnt", 'echo /dev/nvme0n1p5'), ("blkid", 'echo 3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b')):
        (bin_dir / name).write_text(f"#!/bin/sh\n{body}\n")
        (bin_dir / name).chmod(0o755)
    os.environ["PATH"] = f"{bin_dir}:{os.environ['PATH']}"
    try:
        r, target, _ = _run_finish(tmp_path)
    finally:
        os.environ["PATH"] = os.environ["PATH"].split(":", 1)[1]
    assert r.returncode == 0, r.stdout + r.stderr
    marker = (target / "boot/efi/EFI/chatcomputer/installed-0123456789abcdef").read_text()
    assert marker == "id=0123456789abcdef\nroot=3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b\n"


def test_finish_script_leaves_the_rest_to_the_first_start_when_offline(tmp_path):
    r, target, lines = _run_finish(tmp_path, installer_works=False)
    assert r.returncode == 0, r.stdout + r.stderr          # the Ubuntu install still ends normally
    assert sum("install-chatcomputer.sh" in ln for ln in lines) == 3
    assert "finish installing when the computer starts" in r.stdout
    assert (target / "etc/systemd/system/multi-user.target.wants/chatcomputer-finish-install.service").is_symlink()
    assert (target / "boot/efi/EFI/chatcomputer/installed-0123456789abcdef").exists()


def test_finish_script_refuses_bad_user_names_and_ids(tmp_path):
    env = dict(os.environ, CHATCOMPUTER_FINISH_SRC=str(tmp_path), CHATCOMPUTER_FINISH_TARGET=str(tmp_path))
    for args in (["rm -rf /", "0123456789abcdef"], ["sammy", "../../x"], ["sammy", ""]):
        r = subprocess.run(["sh", str(WIN / "installer/finish.sh"), *args], env=env, capture_output=True, text=True)
        assert r.returncode == 2, args


# guard.sh, with folders standing in for the PC's EFI partitions and for the firmware's variables
GLOBAL = "8be4df61-93ca-11d2-aa0d-00e098032b8c"


def _boot_entry(name):
    """A Boot#### variable as efivarfs shows it: variable attributes, entry attributes, path length, name."""
    return (b"\x07\x00\x00\x00" + b"\x01\x00\x00\x00" + b"\x04\x00" + (name + "\0").encode("utf-16-le")
            + b"\x7f\xff\x04\x00")


def _run_guard(tmp_path, installed=True, entries=(("0000", "UiApp"), ("0003", "ubuntu"))):
    esp = tmp_path / "esp"
    (esp / "EFI/chatcomputer").mkdir(parents=True)
    if installed:
        (esp / "EFI/chatcomputer/installed-0123456789abcdef").write_text("0123456789abcdef\n")
    (esp / "EFI/chatcomputer/installed-ffffffffffffffff").write_text("another installer\n")
    efivars = tmp_path / "efivars"
    efivars.mkdir()
    for num, name in entries:
        (efivars / f"Boot{num}-{GLOBAL}").write_bytes(_boot_entry(name))
    restarted = tmp_path / "restarted"
    restart = tmp_path / "restart"
    restart.write_text(f"#!/bin/sh\ntouch {restarted}\n")
    restart.chmod(0o755)
    env = dict(os.environ, CHATCOMPUTER_GUARD_ESPS=f"{tmp_path / 'nothing'} {esp}", CHATCOMPUTER_EFIVARS=str(efivars),
               CHATCOMPUTER_GUARD_REBOOT=str(restart))
    r = subprocess.run(["sh", str(WIN / "installer/guard.sh"), "0123456789abcdef"], env=env, capture_output=True, text=True)
    return r, efivars / f"BootNext-{GLOBAL}", restarted


def test_guard_lets_a_first_install_go_ahead(tmp_path):
    r, boot_next, restarted = _run_guard(tmp_path, installed=False)
    assert r.returncode == 0 and not boot_next.exists() and not restarted.exists()


def test_guard_restarts_into_the_installed_system(tmp_path):
    r, boot_next, restarted = _run_guard(tmp_path, entries=(("0000", "UiApp"), ("0001", "ubuntu2"), ("001A", "ubuntu")))
    assert r.returncode == 0, r.stdout + r.stderr
    assert boot_next.read_bytes() == b"\x07\x00\x00\x00\x1a\x00"   # attributes, then entry 0x001A little-endian
    assert restarted.exists()
    assert "already installed" in r.stdout


def test_guard_stops_the_installer_when_it_cant_restart_into_it(tmp_path):
    r, boot_next, restarted = _run_guard(tmp_path, entries=(("0000", "UiApp"),))
    assert r.returncode == 1 and not boot_next.exists() and not restarted.exists()
    assert "won't install again" in r.stderr


# ------------------------------------------------------------------ Windows compatibility
def _ps_files():
    return sorted([*WIN.glob("*.ps1"), *WIN.glob("lib/*.ps1"), *WIN.glob("tests/*.ps1")])


def test_powershell_sources_are_plain_ascii():
    # Windows PowerShell 5.1 reads BOM-less files as ANSI: curly quotes or dashes would break parsing.
    for f in [*_ps_files(), WIN / "Chat Computer Setup.cmd", WIN / "Chat Computer Own System.cmd"]:
        data = f.read_bytes()
        assert all(b < 128 for b in data), f


def test_launcher_uses_windows_line_endings():
    for name in ("Chat Computer Setup.cmd", "Chat Computer Own System.cmd"):
        data = (WIN / name).read_bytes()
        assert b"\r\n" in data and b"\n" not in data.replace(b"\r\n", b""), name


ZONEINFO = Path("/usr/share/zoneinfo")


@pytest.mark.skipif(not (ZONEINFO / "tzdata.zi").exists(), reason="no tzdata here")
def test_time_zones_are_ones_ubuntu_has():
    # Setup's zone goes to the Ubuntu installer and Chat Computer's settings. Ubuntu 24.04 keeps old names such as
    # Asia/Calcutta only in tzdata-legacy, which isn't installed, so every zone must be a current name. (Run
    # where tzdata-legacy isn't installed either, as on Ubuntu 24.04, this checks exactly that.)
    import re
    zones = re.findall(r"= '([^']+)'", (WIN / "lib/Timezones.ps1").read_text())
    assert len(zones) > 100
    legacy = {"Asia/Calcutta", "Europe/Kiev", "America/Godthab", "Asia/Rangoon", "Asia/Katmandu",
              "America/Buenos_Aires", "America/Indianapolis", "Asia/Saigon"}
    assert not legacy & set(zones)
    missing = [z for z in zones if not (ZONEINFO / z).is_file()]
    assert not missing, missing


@needs_pwsh
def test_windows_powershell_5_1_compatible():
    """Syntax, commands and .NET types must exist in Windows PowerShell 5.1 on Windows 10/11."""
    if PSSA is None:
        pytest.skip("PSScriptAnalyzer not available")
    files = ",".join(f"'{f}'" for f in _ps_files())
    profile = "win-48_x64_10.0.17763.0_5.1.17763.316_x64_4.0.30319.42000_framework"
    script = (
        f"$env:PSModulePath = '{PSSA}' + [IO.Path]::PathSeparator + $env:PSModulePath; "
        "Import-Module PSScriptAnalyzer; "
        "$s = @{ IncludeRules = @('PSUseCompatibleSyntax','PSUseCompatibleCommands','PSUseCompatibleTypes'); "
        "  Rules = @{ PSUseCompatibleSyntax = @{ Enable = $true; TargetVersions = @('5.1') }; "
        f"  PSUseCompatibleCommands = @{{ Enable = $true; TargetProfiles = @('{profile}') }}; "
        f"  PSUseCompatibleTypes = @{{ Enable = $true; TargetProfiles = @('{profile}') }} }} }}; "
        f"$r = foreach ($f in @({files})) {{ Invoke-ScriptAnalyzer -Path $f -Settings $s; "
        "Invoke-ScriptAnalyzer -Path $f -Severity Error }; "
        # Loaded with Add-Type before use (inside try/catch), so 'not available by default' is expected.
        # -DeleteKey belongs to the certificate drive (Cert:), which adds it to Remove-Item in Windows PowerShell
        # 3.0 and later; the analyzer's list only has Remove-Item's own parameters.
        "$r = @($r | Where-Object { $_.Message -notlike '*AccountManagement.UserPrincipal*' -and "
        "  $_.Message -notlike \"*parameter 'DeleteKey' is not available for command 'Remove-Item'*\" }); "
        "$r | ForEach-Object { $_.ScriptName + ':' + $_.Line + ' ' + $_.RuleName + ' ' + $_.Message }; "
        "exit $r.Count"
    )
    r = subprocess.run([str(PWSH), "-NoProfile", "-Command", script], capture_output=True, text=True, timeout=600)
    assert r.returncode == 0, r.stdout + r.stderr


# ------------------------------------------------------------------ the whole conversation, on a pretend PC
DESTRUCTIVE = ("Resize-Partition", "Clear-Disk", "Initialize-Disk", "New-InstallerPartition", "Remove-Partition",
               "Disable-FastStartup", "Suspend-BitLocker", "BitLocker add", "BitLocker delete", "bcdedit", "shutdown",
               "vssadmin", "fsutil", "powercfg")
USER_KEY = "{0b000000-0000-4000-8000-000000000001}"           # the person's own BitLocker recovery password
OUR_KEY = "{2b000000-0000-4000-8000-0000000000c1}"         # the one setup adds for Chat Computer
OUR_KEY_PASSWORD = "123456-234567-345678-456789-567890-678901-789012-890123"
SETUP_ID = "{5e7e0a00-0000-4000-8000-00000000000a}"          # the "Chat Computer setup" entry the pretend bcdedit makes
CHATCOMPUTER_ID = "{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}"   # Chat Computer's own entry, once installed
# The pretend PC's Windows disk before Chat Computer, in MiB: EFI, reserved, C: (950 GB), Windows' recovery partition.
DISK0 = [(1, 1, 260), (2, 261, 16), (3, 277, 950 * 1024), (4, 277 + 950 * 1024, 1024)]
RECOVERY = 277 + 950 * 1024
REMOVE = "Q Type REMOVE to remove Chat Computer (anything else keeps it) -> REMOVE"


def _flow(tmp_path, scenario):
    out = tmp_path / scenario
    r = subprocess.run([str(PWSH), "-NoProfile", "-File", str(WIN / "tests/Flow-Test.ps1"), "-Scenario", scenario,
                        "-OutDir", str(out)], capture_output=True, text=True, timeout=600)
    assert (out / "flow.json").exists(), r.stdout + r.stderr
    res = json.loads((out / "flow.json").read_text())
    assert res["answers_left"] == 0, res                     # every scripted answer was asked for
    res["screen"] = r.stdout
    return out, res


def _disk(res, number):
    """(partition number, start, size) in MiB, in disk order."""
    return [(p["number"], p["start_mib"], p["size_mib"]) for p in res["partitions"] if p["disk"] == number]


def _in_order(calls, *wanted):
    where = [calls.index(c) for c in wanted]
    assert where == sorted(where), wanted


def _changes(calls):
    return [c for c in calls if c.startswith(DESTRUCTIVE)]


@needs_pwsh
def test_flow_plan_only_changes_nothing(tmp_path):
    out, res = _flow(tmp_path, "plan")
    assert res["ok"], res
    assert not _changes(res["calls"]), res["calls"]
    ai = _load(out / "work/plan/autoinstall.yaml")
    assert ai["storage"]["layout"] == {"name": "direct", "mode": "use_gap", "match": {"install-media": True}}


@needs_pwsh
def test_flow_shrink_c_and_make_the_installer(tmp_path):
    out, res = _flow(tmp_path, "shrink")
    assert res["ok"], res
    calls = res["calls"]
    # C: gives 64 GB for Chat Computer plus 6 GB for its installer, without a question about the size. The installer
    # goes at the end of that space, right before Windows' recovery partition; Chat Computer installs into the 64 GB.
    made = f"New-InstallerPartition 0 {RECOVERY - 6144} 6144"
    assert _disk(res, 0) == [*DISK0[:2], (3, 277, 880 * 1024), (5, RECOVERY - 6144, 6144), DISK0[3]]
    # The only questions: where (this pretend PC also has a second disk), "Go ahead?" and "Restart now?".
    assert [c for c in calls if c.startswith("Q ")] == [
        "Q Type a number (Enter = 1) -> 1", "Q Go ahead? [y/N] -> y", "Q Restart now? [Y/n] -> n"]
    # Nothing changes before "Go ahead". Then the power-on entry first (if the PC won't take one, nothing else
    # changes), and only when all is ready, Fast Startup goes off and the PC is told to start the installer once.
    go = "Q Go ahead? [y/N] -> y"
    assert _changes(calls[:calls.index(go)]) == []
    _in_order(calls, go, "bcdedit /copy {bootmgr} /d Chat Computer setup",
              f"bcdedit /set {SETUP_ID} path \\EFI\\chatcomputer-setup\\shimx64.efi",
              f"bcdedit /set {{fwbootmgr}} displayorder {SETUP_ID} /addlast", "Resize-Partition C 880", made,
              "Copy-IsoToInstaller", "Close-InstallerPartition", "Disable-FastStartup",
              f"bcdedit /set {{fwbootmgr}} bootsequence {SETUP_ID}", "Q Restart now? [Y/n] -> n")
    # Plenty of room: nothing else had to be cleared out of the way.
    assert not [c for c in calls if c.startswith(("vssadmin", "fsutil", "powercfg"))]
    assert res["boot_entries"] == {SETUP_ID: "\\EFI\\chatcomputer-setup\\shimx64.efi"}
    assert not [c for c in calls if c.startswith(("Clear-Disk", "Remove-Partition", "shutdown"))]
    # The boot loader that entry starts, on Windows' EFI partition: Ubuntu's signed shim, GRUB and MokManager.
    loader = out / "esp/EFI/chatcomputer-setup"
    assert sorted(f.name for f in loader.iterdir()) == ["grubx64.efi", "mmx64.efi", "shimx64.efi"]
    assert (loader / "shimx64.efi").read_text() == "bootx64.efi"
    inst = out / "installer"
    ai = _load(inst / "autoinstall.yaml")
    if SCHEMA is not None:
        import jsonschema
        jsonschema.validate(ai, json.loads(SCHEMA.read_text()))
    assert ai["storage"]["layout"]["match"] == {"install-media": True}
    install_id = ai["early-commands"][0][2]
    # The mark that tells "Remove Chat Computer", and Chat Computer's first start, that this partition is its installer.
    assert (inst / f"chatcomputer/installer-{install_id}").read_text() == f"id={install_id}\n"
    assert ai["network"]["wifis"]["any-wifi"]["access-points"] == {"Home 5G": {"password": "correct horse"}}
    assert ai["timezone"] == "Asia/Singapore" and ai["identity"]["realname"] == "Sam Lee"
    # Nothing to sign in to: Chat Computer thinks with a model on the PC itself. No secrets go with it here (no
    # BitLocker on this PC).
    assert list((inst / "chatcomputer/secrets").iterdir()) == []
    assert (inst / "chatcomputer/app/chatcomputer/__init__.py").exists()
    assert not (inst / "chatcomputer/app/windows").exists() and not (inst / "chatcomputer/app/.venv").exists()
    # Nor the website, the apps, or what was built here from the own system's source.
    assert not any((inst / "chatcomputer/app" / p).exists() for p in ("docs", "apps", "evals", "os/target", "os/out"))
    assert (inst / "chatcomputer/app/chatcomputer/brain/bin/chat-computer-model-x86_64").exists()
    # Updates come from the folder setup runs from only when it's on Windows' own drive (C:); this pretend PC's
    # setup isn't (Get-UpdateSource, finish.sh).
    assert not (inst / "chatcomputer/update-source").exists()
    for script in ("guard.sh", "network.sh", "finish.sh"):
        assert b"\r" not in (inst / "chatcomputer" / script).read_bytes()
    grub = (inst / "boot/grub/grub.cfg").read_text()
    assert grub.count(" autoinstall ") == 2 and f"/EFI/chatcomputer/installed-{install_id}" in grub
    recovery = (out / "Documents/Chat Computer recovery password.txt").read_text()
    password = recovery.split("Password: ")[1].split()[0]
    log = (out / "setup.log").read_text()
    for secret in ("correct horse", password):
        assert secret not in log, secret


@needs_pwsh
def test_flow_shows_the_recovery_password_when_documents_cant_take_it(tmp_path):
    # By then the installer is ready and the PC starts it at the next restart: a note that couldn't be saved used
    # to end in "Setup stopped", with the password lost.
    out, res = _flow(tmp_path, "nodocs")
    assert res["ok"], res
    assert f"bcdedit /set {{fwbootmgr}} bootsequence {SETUP_ID}" in res["calls"]
    assert res["calls"][-1] == "Q Restart now? [Y/n] -> n"
    shown = re.search(r"Account: sammy   Password: ([a-z2-9]{4}(?:-[a-z2-9]{4}){3})", res["screen"])
    assert shown, res["screen"]
    assert shown.group(1) not in (out / "setup.log").read_text()
    assert not list(out.rglob("Chat Computer recovery password.txt"))


@needs_pwsh
def test_flow_separate_disk(tmp_path):
    out, res = _flow(tmp_path, "disk")
    assert res["ok"], res
    calls = res["calls"]
    assert "Clear-Disk 1" in calls and not [c for c in calls if c.startswith("Resize-Partition")]
    # The erased disk holds just the installer, at its end; Chat Computer installs in the space before it. The
    # Windows disk is untouched, except that the installer's boot loader goes on its EFI partition.
    end = 2000 * 1024 - 1
    assert _disk(res, 1) == [(1, end - 6144, 6144)]
    assert _disk(res, 0) == DISK0
    assert "Mount-EspPartition 0 1" in calls[calls.index("Copy-IsoToInstaller"):]
    assert (out / "esp/EFI/chatcomputer-setup/shimx64.efi").exists()
    assert _load(out / "installer/autoinstall.yaml")["storage"]["layout"]["match"] == {"install-media": True}
    assert (out / "installer/chatcomputer/app/chatcomputer/__init__.py").exists()
    assert calls[-1] == "shutdown /r /t 5"


@needs_pwsh
def test_flow_stops_before_changing_anything_when_the_pc_wont_take_the_entry(tmp_path):
    out, res = _flow(tmp_path, "no-entry")
    assert not res["ok"] and res["error"].endswith("No disks were changed."), res
    assert _changes(res["calls"]) == ["bcdedit /copy {bootmgr} /d Chat Computer setup"]
    assert _disk(res, 0) == DISK0


@needs_pwsh
def test_flow_takes_back_what_it_did_when_a_step_fails(tmp_path):
    out, res = _flow(tmp_path, "install-fails")
    assert not res["ok"], res
    assert "did not copy correctly" in res["error"] and res["error"].endswith("Windows starts normally. C: has its space back.")
    calls = res["calls"]
    _in_order(calls, "Copy-IsoToInstaller", "Close-InstallerPartition", f"bcdedit /delete {SETUP_ID}",
              "Remove-Partition 0 5", "Resize-Partition C 950")
    assert not [c for c in calls if "bootsequence" in c]
    assert res["boot_entries"] == {} and _disk(res, 0) == DISK0
    assert not (out / "esp/EFI/chatcomputer-setup").exists()


@needs_pwsh
def test_flow_wont_install_while_an_earlier_installer_is_there(tmp_path):
    out, res = _flow(tmp_path, "leftover")
    assert not res["ok"] and "An installer from an earlier try is still on this PC" in res["error"], res
    assert not _changes(res["calls"])


@needs_pwsh
def test_flow_stops_when_bitlocker_key_not_saved(tmp_path):
    out, res = _flow(tmp_path, "bitlocker-stop")
    assert not res["ok"] and "recovery key" in res["error"]
    assert not _changes(res["calls"])


@needs_pwsh
def test_flow_gives_chatcomputer_its_own_bitlocker_key(tmp_path):
    out, res = _flow(tmp_path, "bitlocker")
    assert res["ok"], res
    calls = res["calls"]
    # Made after "Go ahead", with the installer, before the PC is told to start it.
    _in_order(calls, "Q Go ahead? [y/N] -> y", "Copy-IsoToInstaller", "BitLocker add key",
              f"bcdedit /set {{fwbootmgr}} bootsequence {SETUP_ID}")
    assert res["bitlocker_keys"] == [USER_KEY, OUR_KEY]          # the person's own key stays
    # Chat Computer gets the key in the installer's secrets (finish.sh puts it in /etc/chatcomputer), and setup keeps
    # its id, so "Remove Chat Computer" deletes exactly this key.
    assert (out / "installer/chatcomputer/secrets/bitlocker-key").read_text() == OUR_KEY_PASSWORD + "\n"
    assert (out / "ProgramData/ChatComputer/bitlocker-key-id").read_text() == OUR_KEY + "\n"
    assert OUR_KEY_PASSWORD not in (out / "setup.log").read_text()


@needs_pwsh
def test_flow_deletes_chatcomputer_bitlocker_key_when_a_later_step_fails(tmp_path):
    out, res = _flow(tmp_path, "bitlocker-fails")
    assert not res["ok"] and "EFI partition" in res["error"], res
    _in_order(res["calls"], "BitLocker add key", f"BitLocker delete key {OUR_KEY}", "Remove-Partition 0 5",
              "Resize-Partition C 950")
    assert res["bitlocker_keys"] == [USER_KEY]
    assert not (out / "ProgramData/ChatComputer/bitlocker-key-id").exists()
    assert _disk(res, 0) == DISK0


@needs_pwsh
def test_flow_makes_room_like_by_hand_when_windows_files_are_in_the_way(tmp_path):
    # The PC setup was first tried on: one disk, 272 GB free on C:, yet Windows could shrink C: by nothing,
    # because a restore point sat at its very end; then the change journal was in the way (twice, as Windows
    # starts a new one), then the file system's own $Mft. Setup does what was done there by hand, after the one
    # "Go ahead?", and installs into the 63 GB it made.
    out, res = _flow(tmp_path, "blocked")
    assert res["ok"], res
    calls = res["calls"]
    assert [c for c in calls if c.startswith("Q ")] == ["Q Go ahead? [y/N] -> y", "Q Restart now? [Y/n] -> n"]
    go = "Q Go ahead? [y/N] -> y"
    assert _changes(calls[:calls.index(go)]) == []
    room = [c for c in calls if c.startswith(("vssadmin", "fsutil", "powercfg", "Resize-Partition"))]
    assert room == ["vssadmin delete shadows /for=C: /quiet", "Resize-Partition C 948", "fsutil usn deletejournal /n C:",
                    "Resize-Partition C 913", "fsutil usn deletejournal /n C:", "Resize-Partition C 887"]
    assert _disk(res, 0) == [*DISK0[:2], (3, 277, 887 * 1024), (5, RECOVERY - 6144, 6144), DISK0[3]]
    _in_order(calls, go, "bcdedit /copy {bootmgr} /d Chat Computer setup", "vssadmin delete shadows /for=C: /quiet",
              f"New-InstallerPartition 0 {RECOVERY - 6144} 6144", f"bcdedit /set {{fwbootmgr}} bootsequence {SETUP_ID}")
    # The list before "Go ahead?" said so.
    log = (out / "setup.log").read_text()
    assert "deletes your restore points" in log and "resets the drive's change journal" in log
    assert "\\$Mft::$BITMAP (ntfs)" in log


@needs_pwsh
def test_flow_uses_space_already_made_after_c(tmp_path):
    # The same PC once the room was made by hand: 63 GB empty after C:, $Mft in the way of more. Nothing is
    # deleted or reset to get the last few GB.
    out, res = _flow(tmp_path, "gap63")
    assert res["ok"], res
    calls = res["calls"]
    assert not [c for c in calls if c.startswith(("vssadmin", "fsutil", "powercfg", "Resize-Partition"))]
    assert _disk(res, 0) == [*DISK0[:2], (3, 277, 887 * 1024), (5, RECOVERY - 6144, 6144), DISK0[3]]


@needs_pwsh
def test_flow_says_what_to_do_when_the_page_file_is_in_the_way(tmp_path):
    out, res = _flow(tmp_path, "stuck")
    assert not res["ok"], res
    err = res["error"]
    assert "page file" in err and "Virtual memory" in err and err.endswith("Windows starts normally. No disks were changed."), err
    assert _disk(res, 0) == DISK0 and res["boot_entries"] == {}
    # It deletes nothing it doesn't need to: restore points stay when they aren't what's in the way.
    assert not [c for c in res["calls"] if c.startswith(("vssadmin", "Disable-FastStartup", "New-InstallerPartition"))]


@needs_pwsh
def test_flow_says_how_much_to_free_up(tmp_path):
    out, res = _flow(tmp_path, "nospace")
    assert not res["ok"], res
    assert "C: has 50 GB free" in res["error"] and "Free up about 21 GB" in res["error"], res["error"]
    assert not _changes(res["calls"])


def _run_network(tmp_path, cfg_text, wifi_iface):
    cfg = tmp_path / "autoinstall.yaml"
    cfg.write_text(cfg_text)
    net = tmp_path / "net"
    for name in ("lo", "enp0s3"):
        (net / name).mkdir(parents=True)
    if wifi_iface:
        (net / wifi_iface / "wireless").mkdir(parents=True)
    env = dict(os.environ, CHATCOMPUTER_AUTOINSTALL=str(cfg), CHATCOMPUTER_NET_SYS=str(net), CHATCOMPUTER_NET_WAIT="1")
    r = subprocess.run(["sh", str(WIN / "installer/network.sh")], env=env, capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    return cfg.read_text(), r.stdout


def test_installer_wifi_gets_the_real_card_name(generated, tmp_path):
    text, out = _run_network(tmp_path, (generated / "autoinstall-wifi.yaml").read_text(), "wlp2s0")
    import yaml
    ai = yaml.safe_load(text)["autoinstall"]
    assert ai["network"]["wifis"] == {"wlp2s0": {"dhcp4": True, "optional": True,
                                                 "access-points": {'Home "5G"': {"password": 'p@ss w0rd"\\'}}}}
    assert ai["network"]["ethernets"]["any-wired"]["match"] == {"name": "en*"}
    assert "wlp2s0" in out
    if SCHEMA is not None:
        import jsonschema
        jsonschema.validate(ai, json.loads(SCHEMA.read_text()))


def test_installer_without_wifi_card_drops_wifi(generated, tmp_path):
    text, out = _run_network(tmp_path, (generated / "autoinstall-wifi.yaml").read_text(), None)
    import yaml
    ai = yaml.safe_load(text)["autoinstall"]
    assert "wifis" not in ai["network"]
    assert ai["identity"]["username"] == "sammy"          # the rest is untouched
    assert ai["late-commands"][0][1] == "/cdrom/chatcomputer/finish.sh"


def test_installer_network_untouched_without_wifi_section(generated, tmp_path):
    original = (generated / "autoinstall-wired.yaml").read_text()
    text, _ = _run_network(tmp_path, original, "wlp2s0")
    assert text == original


# ------------------------------------------------------------------ going back to Windows
@needs_pwsh
def test_windows_can_start_by_default_again(tmp_path):
    out, res = _flow(tmp_path, "startup-windows")
    assert res["ok"], res
    assert _changes(res["calls"]) == ["bcdedit /set {fwbootmgr} displayorder {bootmgr} /addfirst"]


@needs_pwsh
def test_chatcomputer_can_start_once_from_windows(tmp_path):
    out, res = _flow(tmp_path, "startup-once")
    assert res["ok"], res
    assert _changes(res["calls"]) == [f"bcdedit /set {{fwbootmgr}} bootsequence {CHATCOMPUTER_ID}"]


@needs_pwsh
def test_removing_chatcomputer_gives_windows_everything_back(tmp_path):
    out, res = _flow(tmp_path, "remove")
    assert res["ok"], res
    calls = res["calls"]
    # Nothing changes before the typed confirmation; then Windows first, Chat Computer's entry out, its partition
    # (and only that one) deleted, and C: grown back.
    assert not _changes(calls[:calls.index(REMOVE)])
    _in_order(calls, REMOVE, "Suspend-BitLocker C: 1", "bcdedit /set {fwbootmgr} displayorder {bootmgr} /addfirst",
              f"bcdedit /delete {CHATCOMPUTER_ID}", "Remove-Partition 0 5", "Resize-Partition C 950")
    assert [c for c in calls if c.startswith("Remove-Partition")] == ["Remove-Partition 0 5"]
    assert _disk(res, 0) == DISK0
    # Chat Computer's BitLocker key goes too, and only that one.
    _in_order(calls, "Remove-Partition 0 5", f"BitLocker delete key {OUR_KEY}")
    assert res["bitlocker_keys"] == [USER_KEY]
    assert not (out / "ProgramData/ChatComputer/bitlocker-key-id").exists()
    esp = out / "esp/EFI"
    assert not (esp / "ubuntu").exists() and not (esp / "chatcomputer").exists()
    assert (esp / "BOOT/BOOTX64.EFI").read_text() == "Windows Boot Manager"     # Windows back in the fallback slot
    assert not (esp / "BOOT/fbx64.efi").exists()
    assert (esp / "Microsoft/Boot/bootmgfw.efi").exists()
    assert not (out / "Users/sammy/ChatComputer").exists()       # the sign-in copy an earlier version left
    assert not (out / "Documents/Chat Computer recovery password.txt").exists()
    assert not list((out / "work").iterdir())                                    # downloads


@needs_pwsh
def test_removing_chatcomputer_can_keep_the_downloads_to_install_again(tmp_path):
    # Installing again (to get a new version of Chat Computer) then needn't download Ubuntu's 3 GB again. What an
    # earlier version downloaded for Claude (its engine) isn't needed any more, and goes.
    out, res = _flow(tmp_path, "remove-again")
    assert res["ok"], res
    assert [c for c in res["calls"] if c.startswith("Q Keep the downloads")][0].endswith("[y/N] -> y")
    assert sorted(f.name for f in (out / "work").iterdir()) == ["ubuntu-24.04.5-live-server-amd64.iso"]
    assert _disk(res, 0) == DISK0 and not (out / "esp/EFI/chatcomputer").exists()  # Chat Computer itself is gone
    assert not (out / "Users/sammy/ChatComputer").exists()


@needs_pwsh
def test_removing_chatcomputer_needs_the_typed_word(tmp_path):
    out, res = _flow(tmp_path, "remove-keep")
    assert res["ok"], res
    assert not _changes(res["calls"])
    assert (out / "esp/EFI/ubuntu/shimx64.efi").exists() and (out / "esp/EFI/chatcomputer").exists()


@needs_pwsh
def test_removing_chatcomputer_before_it_cleaned_up_its_installer(tmp_path):
    out, res = _flow(tmp_path, "remove-both")
    assert res["ok"], res
    calls = res["calls"]
    _in_order(calls, REMOVE, "bcdedit /set {fwbootmgr} displayorder {bootmgr} /addfirst", f"bcdedit /delete {CHATCOMPUTER_ID}",
              f"bcdedit /delete {SETUP_ID}", "Remove-Partition 0 5", "Remove-Partition 0 6", "Resize-Partition C 950")
    assert _disk(res, 0) == DISK0 and res["boot_entries"] == {}
    assert sorted(d.name for d in (out / "esp/EFI").iterdir()) == ["BOOT", "Microsoft"]
    # The sign-in copy goes; a file of the person's own in that folder (and so the folder) stays.
    assert sorted(f.name for f in (out / "Users/sammy/ChatComputer").iterdir()) == ["notes.txt"]


@needs_pwsh
def test_removing_an_install_that_never_finished(tmp_path):
    out, res = _flow(tmp_path, "remove-unfinished")
    assert res["ok"], res
    calls = res["calls"]
    _in_order(calls, REMOVE, f"bcdedit /delete {SETUP_ID}", "Remove-Partition 0 5", "Resize-Partition C 950")
    assert [c for c in calls if c.startswith(("Remove-Partition", "bcdedit /delete"))] == \
        [f"bcdedit /delete {SETUP_ID}", "Remove-Partition 0 5"]
    assert _disk(res, 0) == DISK0 and res["boot_entries"] == {}
    assert sorted(d.name for d in (out / "esp/EFI").iterdir()) == ["Microsoft"]
