"""mac/setup.sh against a pretend Mac: macOS's commands, uv and the Claude engine are stand-ins that record
what they were asked to do, the home folder is a temporary one, and the answers are typed in advance."""
import os
import plistlib
import shutil
import stat
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
SETUP = REPO / "mac" / "setup.sh"
TOKEN = "sk-ant-oat01-" + "x" * 90

pytestmark = pytest.mark.skipif(shutil.which("bash") is None, reason="needs bash")


def _tool(bin_dir: Path, name: str, body: str) -> None:
    f = bin_dir / name
    f.write_text("#!/bin/bash\n" + body + "\n")
    f.chmod(0o755)


def pretend_mac(tmp_path, version="15.6", arch="arm64", free_kb=50_000_000, apple_silicon_hw=None):
    home = tmp_path / "home"
    home.mkdir()
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    calls = tmp_path / "calls.log"
    rec = f'echo "$(basename "$0") $*" >> "{calls}"'
    _tool(bin_dir, "sw_vers", f'echo {version}')
    _tool(bin_dir, "uname", f'[ "$1" = -m ] && echo {arch} || echo Darwin')
    # sysctl hw.optional.arm64: 1 on Apple silicon (even under Rosetta), missing on Intel Macs
    _tool(bin_dir, "sysctl", 'exit 1' if apple_silicon_hw is None else f'echo {apple_silicon_hw}')
    _tool(bin_dir, "df", f'echo "Filesystem 1024-blocks Used Available Capacity Mounted"; echo "/dev/disk3s5 900000000 1 {free_kb} 5% /"')
    _tool(bin_dir, "curl", rec + "\nexit 0")
    _tool(bin_dir, "id", '[ "$1" = -F ] && { echo "Ren Shu"; exit 0; }; [ "$1" = -u ] && { echo 501; exit 0; }; exec /usr/bin/id "$@"')
    for name in ("xattr", "codesign", "launchctl", "open", "pkill"):
        _tool(bin_dir, name, rec)
    # uv: makes a real (small) Python environment, so the .pth and import check really run; installs nothing.
    # It notes which install_name_tool it would find (uv runs it after downloading Python).
    _tool(bin_dir, "fake-uv", rec + f'\nif [ "$1" = venv ]; then echo "install_name_tool: $(command -v install_name_tool)" >> "{calls}"; '
          f'"{sys.executable}" -m venv --without-pip "${{@: -1}}"; fi')
    # The Claude engine: prints a sign-in code, and says a full claude.ai sign-in is present after login.
    _tool(bin_dir, "fake-claude", rec + f'''
case "$1 $2" in
  "setup-token ") echo "Your token: {TOKEN}" ;;
  "auth status") echo '{{"loggedIn": true, "authMethod": "claude.ai"}}' ;;
esac''')
    env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", CLAUDEOS_MAC_ROOT=str(home),
               CLAUDEOS_UV=str(bin_dir / "fake-uv"), CLAUDEOS_ENGINE_CLI=str(bin_dir / "fake-claude"),
               CLAUDEOS_SKIP_MODELS="1")
    return home, env, calls


# The shell setup runs in: macOS's is bash 3.2 (CLAUDEOS_TEST_BASH can point at one).
SHELL = os.environ.get("CLAUDEOS_TEST_BASH") or "bash"


def run_setup(env, answers):
    return subprocess.run([SHELL, str(SETUP)], input="\n".join(answers) + "\n", env=env, capture_output=True,
                          text=True, timeout=180)


def test_install_on_a_mac(tmp_path):
    home, env, calls = pretend_mac(tmp_path)
    # Install, go ahead, paste the code, phone typing yes, open at login yes, don't start now.
    r = run_setup(env, ["1", "y", TOKEN, "y", "y", "n"])
    assert r.returncode == 0, r.stdout + r.stderr
    co = home / "ClaudeOS"
    assert (co / "app/claudeos/__init__.py").exists() and (co / "app/mac/requirements-mac.txt").exists()
    pth = list((co / "runtime").glob("lib/python*/site-packages/claudeos-app.pth"))
    assert pth and pth[0].read_text().strip() == str(co / "app")
    token = co / "secrets/claude_token"
    assert token.read_text().strip() == TOKEN and stat.S_IMODE(token.stat().st_mode) == 0o600
    assert stat.S_IMODE((co / "secrets").stat().st_mode) == 0o700
    assert '"user_name": "Ren Shu"' in (co / "settings.json").read_text()
    app = home / "Applications/ClaudeOS.app/Contents"
    info = plistlib.loads((app / "Info.plist").read_bytes())
    assert info["CFBundleExecutable"] == "ClaudeOS" and "NSMicrophoneUsageDescription" in info
    # The app is the one built and signed ahead of time, copied as it is (so its signature still holds).
    built = REPO / "mac/ClaudeOS.app/Contents"
    for f in ("Info.plist", "MacOS/ClaudeOS", "Resources/ClaudeOS.icns", "_CodeSignature/CodeResources"):
        assert (app / f).read_bytes() == (built / f).read_bytes(), f
    assert os.access(app / "MacOS/ClaudeOS", os.X_OK)
    conf = dict(line.split("=", 1) for line in (co / "launch.conf").read_text().splitlines())
    assert conf["python"] == str(co / "runtime/bin/python")
    assert Path(conf["libpython"]).is_file() and "libpython" in Path(conf["libpython"]).name
    agent = plistlib.loads((home / "Library/LaunchAgents/com.claudeos.start.plist").read_bytes())
    assert agent["ProgramArguments"] == ["/usr/bin/open", "-a", str(home / "Applications/ClaudeOS.app")]
    log = calls.read_text()
    assert "fake-claude auth login --claudeai" in log and "fake-uv pip install" in log
    assert "--only-binary :all:" in log            # nothing is compiled on the Mac
    assert "codesign" not in log                   # nothing is signed on the Mac (it needs developer tools)
    assert f"install_name_tool: {co}/.setup/bin/install_name_tool" in log      # uv gets the do-nothing one
    assert "open " not in log                      # "don't start now"
    assert TOKEN not in (co / "logs/setup.log").read_text()


def test_an_old_mac_is_turned_away_before_anything_changes(tmp_path):
    home, env, _ = pretend_mac(tmp_path, version="13.6", arch="arm64")
    r = run_setup(env, ["1"])
    assert r.returncode == 1 and "macOS 14" in r.stdout
    assert not (home / "ClaudeOS/app").exists() and not (home / "Applications").exists()


def test_an_intel_mac_on_macos_13_is_fine(tmp_path):
    home, env, _ = pretend_mac(tmp_path, version="13.7", arch="x86_64")
    r = run_setup(env, ["1", "n"])                # checks pass, then "Go ahead?" no
    assert r.returncode == 0 and "Nothing was changed" in r.stdout and "Intel" in r.stdout
    assert not (home / "ClaudeOS").exists()       # just looking leaves nothing behind


def test_apple_silicon_under_rosetta_counts_as_apple_silicon(tmp_path):
    home, env, _ = pretend_mac(tmp_path, version="13.7", arch="x86_64", apple_silicon_hw=1)
    r = run_setup(env, ["1"])
    assert r.returncode == 1 and "macOS 14" in r.stdout


def test_removing_when_nothing_is_installed(tmp_path):
    home, env, _ = pretend_mac(tmp_path)
    r = run_setup(env, ["4"])
    assert r.returncode == 0 and "isn't installed" in r.stdout and not (home / "ClaudeOS").exists()


def test_not_opening_at_login_and_removing(tmp_path):
    home, env, calls = pretend_mac(tmp_path)
    assert run_setup(env, ["1", "y", TOKEN, "n", "y", "n"]).returncode == 0
    agent = home / "Library/LaunchAgents/com.claudeos.start.plist"
    assert agent.exists()
    r = run_setup(env, ["3", "2"])                # just macOS at login
    assert r.returncode == 0 and not agent.exists() and "launchctl bootout" in calls.read_text()
    (home / "ClaudeOS/memory").mkdir(exist_ok=True)
    (home / "ClaudeOS/memory/memory.md").write_text("Likes jazz.")
    r = run_setup(env, ["4", "keep it"])          # not confirmed
    assert "Nothing was changed" in r.stdout and (home / "ClaudeOS/runtime").exists()
    r = run_setup(env, ["4", "REMOVE", "y"])      # removed, keeping what it remembers
    assert r.returncode == 0, r.stdout + r.stderr
    co = home / "ClaudeOS"
    assert (co / "memory/memory.md").read_text() == "Likes jazz."
    assert not (co / "runtime").exists() and not (co / "secrets").exists() and not (co / "app").exists()
    assert not (home / "Applications/ClaudeOS.app").exists()
    r = run_setup(env, ["4", "REMOVE", "n"])      # and then everything
    assert r.returncode == 0 and not co.exists()


def test_removing_signs_the_phone_sign_in_out(tmp_path):
    home, env, calls = pretend_mac(tmp_path)
    assert run_setup(env, ["1", "y", TOKEN, "y", "n", "n"]).returncode == 0     # with typing from the phone
    r = run_setup(env, ["4", "REMOVE", "n"])
    assert r.returncode == 0, r.stdout + r.stderr
    assert "fake-claude auth logout" in calls.read_text() and "Keychain" not in r.stdout
    assert not (home / "ClaudeOS").exists() and not (home / "Applications/ClaudeOS.app").exists()


def _install_through_a_terminal(tmp_path, paste):
    """Runs setup in a real terminal (a pty), the way Terminal hands input over; `paste(write)` types the code."""
    import pty
    import select
    import time
    home, env, calls = pretend_mac(tmp_path)
    master, slave = pty.openpty()
    p = subprocess.Popen([SHELL, str(SETUP)], stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    os.close(slave)
    out = b""

    def wait_for(text, timeout=60):
        nonlocal out
        end = time.time() + timeout
        while text.encode() not in out:
            assert time.time() < end, f"never saw {text!r}:\n{out.decode(errors='replace')}"
            r, _, _ = select.select([p.stdout], [], [], 0.2)
            if r:
                out += os.read(p.stdout.fileno(), 65536)

    def write(data, pause=0.0):
        os.write(master, data)
        time.sleep(pause)

    try:
        wait_for("Type a number")
        write(b"1\n")
        wait_for("Go ahead?")
        write(b"y\n")
        wait_for("Paste the code here")
        paste(write)
        wait_for("Got it.")
        wait_for("Set up typing from your phone?")
        write(b"n\n")
        wait_for("Open ClaudeOS when you log in")
        write(b"n\n")
        wait_for("Start ClaudeOS now?")
        write(b"n\n")
        assert p.wait(timeout=60) == 0
    finally:
        if p.poll() is None:
            p.kill()
        os.close(master)
    assert b"\033[?2004h" in out and b"\033[?2004l" in out        # paste marks asked for, then turned off
    co = home / "ClaudeOS"
    assert (co / "secrets/claude_token").read_text().strip() == TOKEN
    setup_log = (co / "logs/setup.log").read_text()
    assert TOKEN[:30] not in setup_log and TOKEN[75:] not in setup_log
    # The Enter pressed after the paste ended it; it didn't answer the next question.
    assert "Q: Set up typing from your phone? (your browser opens once more) -> n" in setup_log
    assert "auth login" not in calls.read_text()


def test_a_code_pasted_over_two_lines_with_paste_marks(tmp_path):
    # Terminal marks the paste; Claude Code's line break and the copy's last line break are inside it.
    def paste(write):
        write(b"\033[200~" + TOKEN[:70].encode() + b"\r" + TOKEN[70:].encode() + b"\r\033[201~", 1.5)
        write(b"\r")                                              # the Enter, a moment later
    _install_through_a_terminal(tmp_path, paste)


def test_a_code_pasted_over_two_lines_without_paste_marks(tmp_path):
    # A terminal that doesn't mark pastes: the second line arrives at once, the Enter seconds later.
    def paste(write):
        write(TOKEN[:70].encode() + b"\r", 0.05)
        write(TOKEN[70:].encode(), 2.5)
        write(b"\r")
    _install_through_a_terminal(tmp_path, paste)
