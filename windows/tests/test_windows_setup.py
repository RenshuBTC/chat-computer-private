"""Checks for ClaudeOS Setup (the Windows tool) that can run on Linux.

* The PowerShell unit tests (windows/tests/Run-Tests.ps1) under pwsh.
* The generated installer answers against the Ubuntu installer's own schema.
* The password hashes against Linux's crypt.
* The two scripts that run inside the installer (shellcheck, and finish.sh against a fake system).
* Windows PowerShell 5.1 compatibility (PSScriptAnalyzer) and plain-ASCII sources.

Tool locations can be given with CLAUDEOS_PWSH, CLAUDEOS_PSSA (folder containing the
PSScriptAnalyzer module) and CLAUDEOS_SUBIQUITY_SCHEMA; tests that need a missing tool are skipped.
"""
import json
import os
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

WIN = Path(__file__).resolve().parents[1]
REPO = WIN.parent
SCRATCH = Path("/tmp/claude-0/-home-claude/3e6478db-57d3-52bd-b4d8-cfceab0784d7/scratchpad/windows-setup")


def _find(env, *candidates):
    v = os.environ.get(env)
    if v and Path(v).exists():
        return Path(v)
    for c in candidates:
        if c and Path(c).exists():
            return Path(c)
    return None


PWSH = _find("CLAUDEOS_PWSH", shutil.which("pwsh"), SCRATCH / "tools/pwsh/pwsh")
PSSA = _find("CLAUDEOS_PSSA", SCRATCH / "tools/modules")
SCHEMA = _find("CLAUDEOS_SUBIQUITY_SCHEMA", SCRATCH / "dl/subiquity-noble/autoinstall-schema.json")
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


def test_answers_only_use_free_space_and_run_claudeos_steps(generated):
    ai = _load(generated / "autoinstall-wifi.yaml")
    # Free space only, on the disk the installer itself is on (setup put it on the disk ClaudeOS goes on).
    assert ai["storage"]["layout"] == {"name": "direct", "mode": "use_gap", "match": {"install-media": True}}
    assert ai["early-commands"] == [["sh", "/cdrom/claudeos/guard.sh", "0123456789abcdef01234567"],
                                    ["sh", "/cdrom/claudeos/network.sh"]]
    assert ai["late-commands"] == [["sh", "/cdrom/claudeos/finish.sh", "rensh", "0123456789abcdef01234567"]]
    assert ai["identity"]["username"] == "rensh"
    assert ai["identity"]["realname"] == 'Ren Shu "é"'
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
    assert text.index('menuentry "Install ClaudeOS" {') < text.index('menuentry "Install ClaudeOS (older kernel)" {')
    # Once installed (finish.sh's marker is on the PC), the installer starts ClaudeOS instead of installing again.
    marker = "/EFI/claudeos/installed-0123456789abcdef01234567"
    assert text.index(f"if search --no-floppy --file --set=claudeos_esp {marker}; then") \
        < text.index('menuentry "Start ClaudeOS') < text.index("\nelse\n") < text.index('menuentry "Install ClaudeOS" {')
    assert "\r" not in text


@pytest.mark.skipif(shutil.which("grub-script-check") is None, reason="grub-script-check not available")
def test_boot_menu_is_valid_grub_script(generated):
    r = subprocess.run(["grub-script-check", str(generated / "grub.cfg")], capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr


def test_settings_json(generated):
    data = json.loads((generated / "settings.json").read_text(encoding="utf-8"))
    assert data == {"user_name": 'Ren "R" Shu', "windows_user": "rensh", "timezone": "Asia/Singapore",
                    "locale": "en_US.UTF-8"}


# ------------------------------------------------------------------ scripts in the installer partition
@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not available")
def test_installer_scripts_shellcheck():
    r = subprocess.run(["shellcheck", "-s", "sh", str(WIN / "installer/guard.sh"), str(WIN / "installer/network.sh"),
                        str(WIN / "installer/finish.sh")],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stdout


def _run_finish(tmp_path, installer_works=True):
    """finish.sh against a fake installer partition and a fake new system; curtin is a stand-in that records its calls."""
    src = tmp_path / "cdrom/claudeos"
    target = tmp_path / "target"
    (src / "app/linux/session").mkdir(parents=True)
    (src / "app/linux/firstboot").mkdir(parents=True)
    (src / "app/claudeos").mkdir(parents=True)
    (src / "app/linux/install-claudeos.sh").write_text("#!/bin/bash\necho install\n")
    for unit in ("claudeos-finish-install.service", "claudeos-reclaim-installer.service"):
        (src / "app/linux/firstboot" / unit).write_text((REPO / "linux/firstboot" / unit).read_text())
    (src / "app/linux/session/claudeos-session").write_text("#!/bin/bash\n")
    (src / "app/linux/session/xinitrc").write_text("exec something\n")
    (src / "app/claudeos/__init__.py").write_text("")
    (src / "secrets").mkdir()
    (src / "secrets/claude_token").write_text("sk-ant-oat01-test\n")
    (src / "secrets/claude_login.json").write_text('{"claudeAiOauth": {"accessToken": "a", "refreshToken": "r"}}')
    (src / "settings.json").write_text("{}\n")
    for p in src.rglob("*"):
        if p.is_file():
            p.chmod(0o777)        # FAT32 shows every file as executable
    (target / "home/rensh").mkdir(parents=True)
    (target / "boot/efi/EFI").mkdir(parents=True)
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    calls = tmp_path / "curtin-calls"
    fail = 'case "$*" in *install-claudeos.sh*) exit 1 ;; esac\n' if not installer_works else ""
    fake = bin_dir / "curtin"
    fake.write_text(f'#!/bin/sh\necho "$@" >> "{calls}"\n{fail}exit 0\n')
    fake.chmod(0o755)
    env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", CLAUDEOS_FINISH_RETRY_WAIT="0",
               CLAUDEOS_FINISH_SRC=str(src), CLAUDEOS_FINISH_TARGET=str(target))
    r = subprocess.run(["sh", str(WIN / "installer/finish.sh"), "rensh", "0123456789abcdef"], env=env,
                       capture_output=True, text=True)
    return r, target, calls.read_text().splitlines()


def test_finish_script_against_a_fake_system(tmp_path):
    r, target, lines = _run_finish(tmp_path)
    assert r.returncode == 0, r.stdout + r.stderr
    app = target / "opt/claudeos/app"
    assert (app / "linux/install-claudeos.sh").exists()
    mode = lambda p: stat.S_IMODE(p.stat().st_mode)  # noqa: E731
    assert mode(app / "linux/session/claudeos-session") == 0o755
    assert mode(app / "linux/session/xinitrc") == 0o644
    assert mode(app / "claudeos/__init__.py") == 0o644
    token = target / "home/rensh/ClaudeOS/secrets/claude_token"
    assert token.read_text() == "sk-ant-oat01-test\n" and mode(token) == 0o600
    assert mode(target / "home/rensh/ClaudeOS/secrets") == 0o700
    login = target / "home/rensh/ClaudeOS/.engine/.credentials.json"
    assert "refreshToken" in login.read_text() and mode(login) == 0o600
    assert mode(target / "home/rensh/ClaudeOS/.engine") == 0o700
    assert mode(target / "home/rensh/ClaudeOS/settings.json") == 0o644
    # The installer's boot menu (and guard.sh) look for this file to tell that this installer already installed here.
    assert (target / "boot/efi/EFI/claudeos/installed-0123456789abcdef").read_text().splitlines()[0] == "id=0123456789abcdef"
    assert any("chown -R rensh:rensh /home/rensh/ClaudeOS" in ln for ln in lines)
    # The first-start safety net is in place (enabled = the wants link)...
    assert (target / "etc/claudeos/install-user").read_text() == "rensh\n"
    assert (target / "etc/systemd/system/claudeos-finish-install.service").exists()
    link = target / "etc/systemd/system/multi-user.target.wants/claudeos-finish-install.service"
    assert os.readlink(link) == "/etc/systemd/system/claudeos-finish-install.service"
    # ...and the long step ran once.
    assert sum("install-claudeos.sh --user rensh --from-installer" in ln for ln in lines) == 1
    # The first start's clean-up of the installer partition: enabled, and told which installer is this one's.
    assert (target / "etc/claudeos/install-id").read_text() == "0123456789abcdef\n"
    assert (target / "etc/systemd/system/claudeos-reclaim-installer.service").exists()
    link = target / "etc/systemd/system/multi-user.target.wants/claudeos-reclaim-installer.service"
    assert os.readlink(link) == "/etc/systemd/system/claudeos-reclaim-installer.service"


def test_finish_script_names_the_claudeos_partition(tmp_path):
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
    marker = (target / "boot/efi/EFI/claudeos/installed-0123456789abcdef").read_text()
    assert marker == "id=0123456789abcdef\nroot=3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b\n"


def test_finish_script_leaves_the_rest_to_the_first_start_when_offline(tmp_path):
    r, target, lines = _run_finish(tmp_path, installer_works=False)
    assert r.returncode == 0, r.stdout + r.stderr          # the Ubuntu install still ends normally
    assert sum("install-claudeos.sh" in ln for ln in lines) == 3
    assert "finish installing when the computer starts" in r.stdout
    assert (target / "etc/systemd/system/multi-user.target.wants/claudeos-finish-install.service").is_symlink()
    assert (target / "boot/efi/EFI/claudeos/installed-0123456789abcdef").exists()


def test_finish_script_refuses_bad_user_names_and_ids(tmp_path):
    env = dict(os.environ, CLAUDEOS_FINISH_SRC=str(tmp_path), CLAUDEOS_FINISH_TARGET=str(tmp_path))
    for args in (["rm -rf /", "0123456789abcdef"], ["rensh", "../../x"], ["rensh", ""]):
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
    (esp / "EFI/claudeos").mkdir(parents=True)
    if installed:
        (esp / "EFI/claudeos/installed-0123456789abcdef").write_text("0123456789abcdef\n")
    (esp / "EFI/claudeos/installed-ffffffffffffffff").write_text("another installer\n")
    efivars = tmp_path / "efivars"
    efivars.mkdir()
    for num, name in entries:
        (efivars / f"Boot{num}-{GLOBAL}").write_bytes(_boot_entry(name))
    restarted = tmp_path / "restarted"
    restart = tmp_path / "restart"
    restart.write_text(f"#!/bin/sh\ntouch {restarted}\n")
    restart.chmod(0o755)
    env = dict(os.environ, CLAUDEOS_GUARD_ESPS=f"{tmp_path / 'nothing'} {esp}", CLAUDEOS_EFIVARS=str(efivars),
               CLAUDEOS_GUARD_REBOOT=str(restart))
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
    for f in [*_ps_files(), WIN / "ClaudeOS Setup.cmd"]:
        data = f.read_bytes()
        assert all(b < 128 for b in data), f


def test_launcher_uses_windows_line_endings():
    data = (WIN / "ClaudeOS Setup.cmd").read_bytes()
    assert b"\r\n" in data and b"\n" not in data.replace(b"\r\n", b"")


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
        "$r = @($r | Where-Object { $_.Message -notlike '*AccountManagement.UserPrincipal*' }); "
        "$r | ForEach-Object { $_.ScriptName + ':' + $_.Line + ' ' + $_.RuleName + ' ' + $_.Message }; "
        "exit $r.Count"
    )
    r = subprocess.run([str(PWSH), "-NoProfile", "-Command", script], capture_output=True, text=True, timeout=600)
    assert r.returncode == 0, r.stdout + r.stderr


# ------------------------------------------------------------------ the whole conversation, on a pretend PC
DESTRUCTIVE = ("Resize-Partition", "Clear-Disk", "Initialize-Disk", "New-InstallerPartition", "Remove-Partition",
               "Disable-FastStartup", "Suspend-BitLocker", "bcdedit", "shutdown")
SETUP_ID = "{5e7e0a00-0000-4000-8000-00000000000a}"          # the "ClaudeOS setup" entry the pretend bcdedit makes
CLAUDEOS_ID = "{9f62d3e2-5c1a-11ef-a1b2-806e6f6e6963}"       # ClaudeOS's own entry, once installed
# The pretend PC's Windows disk before ClaudeOS, in MiB: EFI, reserved, C: (950 GB), Windows' recovery partition.
DISK0 = [(1, 1, 260), (2, 261, 16), (3, 277, 950 * 1024), (4, 277 + 950 * 1024, 1024)]
RECOVERY = 277 + 950 * 1024
REMOVE = "Q Type REMOVE to remove ClaudeOS (anything else keeps it) -> REMOVE"


def _flow(tmp_path, scenario):
    out = tmp_path / scenario
    r = subprocess.run([str(PWSH), "-NoProfile", "-File", str(WIN / "tests/Flow-Test.ps1"), "-Scenario", scenario,
                        "-OutDir", str(out)], capture_output=True, text=True, timeout=600)
    assert (out / "flow.json").exists(), r.stdout + r.stderr
    res = json.loads((out / "flow.json").read_text())
    assert res["answers_left"] == 0, res                     # every scripted answer was asked for
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
    # C: gives 80 GB for ClaudeOS plus 6 GB for its installer. The installer goes at the end of that space,
    # right before Windows' recovery partition; ClaudeOS installs into the 80 GB before it.
    made = f"New-InstallerPartition 0 {RECOVERY - 6144} 6144"
    assert _disk(res, 0) == [*DISK0[:2], (3, 277, 864 * 1024), (5, RECOVERY - 6144, 6144), DISK0[3]]
    # Nothing changes before "Go ahead" but Fast Startup. Then the power-on entry first (if the PC won't take
    # one, nothing else changes), and only when all is ready, the PC is told to start the installer once.
    go = "Q Go ahead? [y/N] -> y"
    assert _changes(calls[:calls.index(go)]) == ["Disable-FastStartup"]
    _in_order(calls, go, "bcdedit /copy {bootmgr} /d ClaudeOS setup",
              f"bcdedit /set {SETUP_ID} path \\EFI\\claudeos-setup\\shimx64.efi",
              f"bcdedit /set {{fwbootmgr}} displayorder {SETUP_ID} /addlast", "Resize-Partition C 864", made,
              "Copy-IsoToInstaller", "Close-InstallerPartition", f"bcdedit /set {{fwbootmgr}} bootsequence {SETUP_ID}",
              "Q Restart now? [Y/n] -> n")
    assert res["boot_entries"] == {SETUP_ID: "\\EFI\\claudeos-setup\\shimx64.efi"}
    assert not [c for c in calls if c.startswith(("Clear-Disk", "Remove-Partition", "shutdown"))]
    # The boot loader that entry starts, on Windows' EFI partition: Ubuntu's signed shim, GRUB and MokManager.
    loader = out / "esp/EFI/claudeos-setup"
    assert sorted(f.name for f in loader.iterdir()) == ["grubx64.efi", "mmx64.efi", "shimx64.efi"]
    assert (loader / "shimx64.efi").read_text() == "bootx64.efi"
    inst = out / "installer"
    ai = _load(inst / "autoinstall.yaml")
    if SCHEMA is not None:
        import jsonschema
        jsonschema.validate(ai, json.loads(SCHEMA.read_text()))
    assert ai["storage"]["layout"]["match"] == {"install-media": True}
    install_id = ai["early-commands"][0][2]
    # The mark that tells "Remove ClaudeOS", and ClaudeOS's first start, that this partition is its installer.
    assert (inst / f"claudeos/installer-{install_id}").read_text() == f"id={install_id}\n"
    assert ai["network"]["wifis"]["any-wifi"]["access-points"] == {"Home 5G": {"password": "correct horse"}}
    assert ai["timezone"] == "Asia/Singapore" and ai["identity"]["realname"] == "Ren Shu"
    assert (inst / "claudeos/secrets/claude_token").read_text().startswith("sk-ant-oat01-")
    # The full sign-in for the Claude app on the phone, asked after the token and before "Go ahead?".
    login = json.loads((inst / "claudeos/secrets/claude_login.json").read_text())
    assert login["claudeAiOauth"]["refreshToken"].startswith("r")
    _in_order(calls, "Invoke-ClaudeSignIn", "Invoke-ClaudeLogin", go)
    assert (inst / "claudeos/app/claudeos/__init__.py").exists()
    assert not (inst / "claudeos/app/windows").exists() and not (inst / "claudeos/app/.venv").exists()
    for script in ("guard.sh", "network.sh", "finish.sh"):
        assert b"\r" not in (inst / "claudeos" / script).read_bytes()
    grub = (inst / "boot/grub/grub.cfg").read_text()
    assert grub.count(" autoinstall ") == 2 and f"/EFI/claudeos/installed-{install_id}" in grub
    recovery = (out / "Documents/ClaudeOS recovery password.txt").read_text()
    password = recovery.split("Password: ")[1].split()[0]
    log = (out / "setup.log").read_text()
    for secret in ("sk-ant-oat01-", "correct horse", password, "a" * 40, "r" * 40):
        assert secret not in log, secret


@needs_pwsh
def test_flow_separate_disk(tmp_path):
    out, res = _flow(tmp_path, "disk")
    assert res["ok"], res
    calls = res["calls"]
    assert "Clear-Disk 1" in calls and not [c for c in calls if c.startswith("Resize-Partition")]
    # The erased disk holds just the installer, at its end; ClaudeOS installs in the space before it. The
    # Windows disk is untouched, except that the installer's boot loader goes on its EFI partition.
    end = 2000 * 1024 - 1
    assert _disk(res, 1) == [(1, end - 6144, 6144)]
    assert _disk(res, 0) == DISK0
    assert "Mount-EspPartition 0 1" in calls[calls.index("Copy-IsoToInstaller"):]
    assert (out / "esp/EFI/claudeos-setup/shimx64.efi").exists()
    assert _load(out / "installer/autoinstall.yaml")["storage"]["layout"]["match"] == {"install-media": True}
    # No phone this time: no full sign-in in the installer.
    assert "Invoke-ClaudeLogin" not in calls
    assert not (out / "installer/claudeos/secrets/claude_login.json").exists()
    assert calls[-1] == "shutdown /r /t 5"


@needs_pwsh
def test_flow_stops_before_changing_anything_when_the_pc_wont_take_the_entry(tmp_path):
    out, res = _flow(tmp_path, "no-entry")
    assert not res["ok"] and res["error"].endswith("No disks were changed."), res
    assert _changes(res["calls"]) == ["Disable-FastStartup", "bcdedit /copy {bootmgr} /d ClaudeOS setup"]
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
    assert not (out / "esp/EFI/claudeos-setup").exists()


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
def test_flow_sign_in_again_leaves_the_token_for_claudeos(tmp_path):
    out, res = _flow(tmp_path, "signin")
    assert res["ok"], res
    assert "Invoke-ClaudeSignIn" in res["calls"]
    assert not _changes(res["calls"])
    tok = (out / "Users/rensh/ClaudeOS/claude_token").read_text()
    assert tok.startswith("sk-ant-oat01-") and tok.endswith("\n")
    assert "Invoke-ClaudeLogin" in res["calls"]
    assert json.loads((out / "Users/rensh/ClaudeOS/claude_login.json").read_text())["claudeAiOauth"]


def _run_network(tmp_path, cfg_text, wifi_iface):
    cfg = tmp_path / "autoinstall.yaml"
    cfg.write_text(cfg_text)
    net = tmp_path / "net"
    for name in ("lo", "enp0s3"):
        (net / name).mkdir(parents=True)
    if wifi_iface:
        (net / wifi_iface / "wireless").mkdir(parents=True)
    env = dict(os.environ, CLAUDEOS_AUTOINSTALL=str(cfg), CLAUDEOS_NET_SYS=str(net), CLAUDEOS_NET_WAIT="1")
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
    assert ai["identity"]["username"] == "rensh"          # the rest is untouched
    assert ai["late-commands"][0][1] == "/cdrom/claudeos/finish.sh"


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
def test_claudeos_can_start_once_from_windows(tmp_path):
    out, res = _flow(tmp_path, "startup-once")
    assert res["ok"], res
    assert _changes(res["calls"]) == [f"bcdedit /set {{fwbootmgr}} bootsequence {CLAUDEOS_ID}"]


@needs_pwsh
def test_removing_claudeos_gives_windows_everything_back(tmp_path):
    out, res = _flow(tmp_path, "remove")
    assert res["ok"], res
    calls = res["calls"]
    # Nothing changes before the typed confirmation; then Windows first, ClaudeOS's entry out, its partition
    # (and only that one) deleted, and C: grown back.
    assert not _changes(calls[:calls.index(REMOVE)])
    _in_order(calls, REMOVE, "Suspend-BitLocker C: 1", "bcdedit /set {fwbootmgr} displayorder {bootmgr} /addfirst",
              f"bcdedit /delete {CLAUDEOS_ID}", "Remove-Partition 0 5", "Resize-Partition C 950")
    assert [c for c in calls if c.startswith("Remove-Partition")] == ["Remove-Partition 0 5"]
    assert _disk(res, 0) == DISK0
    esp = out / "esp/EFI"
    assert not (esp / "ubuntu").exists() and not (esp / "claudeos").exists()
    assert (esp / "BOOT/BOOTX64.EFI").read_text() == "Windows Boot Manager"     # Windows back in the fallback slot
    assert not (esp / "BOOT/fbx64.efi").exists()
    assert (esp / "Microsoft/Boot/bootmgfw.efi").exists()
    assert not (out / "Users/rensh/ClaudeOS").exists()                           # sign-in copies
    assert not (out / "Documents/ClaudeOS recovery password.txt").exists()
    assert not list((out / "work").iterdir())                                    # downloads


@needs_pwsh
def test_removing_claudeos_needs_the_typed_word(tmp_path):
    out, res = _flow(tmp_path, "remove-keep")
    assert res["ok"], res
    assert not _changes(res["calls"])
    assert (out / "esp/EFI/ubuntu/shimx64.efi").exists() and (out / "esp/EFI/claudeos").exists()


@needs_pwsh
def test_removing_claudeos_before_it_cleaned_up_its_installer(tmp_path):
    out, res = _flow(tmp_path, "remove-both")
    assert res["ok"], res
    calls = res["calls"]
    _in_order(calls, REMOVE, "bcdedit /set {fwbootmgr} displayorder {bootmgr} /addfirst", f"bcdedit /delete {CLAUDEOS_ID}",
              f"bcdedit /delete {SETUP_ID}", "Remove-Partition 0 5", "Remove-Partition 0 6", "Resize-Partition C 950")
    assert _disk(res, 0) == DISK0 and res["boot_entries"] == {}
    assert sorted(d.name for d in (out / "esp/EFI").iterdir()) == ["BOOT", "Microsoft"]


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
