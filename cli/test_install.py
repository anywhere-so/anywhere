#!/usr/bin/env python3
"""cli/install.sh, piped into sh as `curl -fsSL .../install.sh | sh` runs it, against a
local server holding the CLI's folder as the control plane serves it: install.sh and,
for each target, a stand-in build with its .sha256. A stand-in `uname`, and `sw_vers`
for macOS's version, name the computer, so every target, and every refusal, runs on any
machine: macOS, the Linux VM and CI. The stand-in build answers `--version` and `skill install` as the real
CLI does. The real CLI is installed from the local stack by the verify-anywhere skill.
"""
import functools
import hashlib
import http.server
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import unittest

SCRIPT = Path(__file__).resolve().with_name("install.sh")
TARGETS = {
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
    ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
}
# What the script runs besides the CLI and sh's own built-ins; each test's PATH
# holds only these, so it never finds a tool by accident.
TOOLS = ("curl", "mktemp", "mkdir", "rm", "chmod", "mv", "cat")
# The script uses whichever of these it finds; Linux has sha256sum, and macOS both.
HASHERS = ("sha256sum", "shasum")

# A stand-in build: its version names its target, and it writes each command it
# runs to $CALLS. `skill install` exits with $SKILL, 0 unless set.
BUILD = """#!/bin/sh
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in
  --version) echo "anywhere 1.2.3 ({target})" ;;
  "skill install") echo "installed the skill"; exit "${{SKILL:-0}}" ;;
  *) echo "unknown command: $*" >&2; exit 2 ;;
esac
"""
UNAME = """#!/bin/sh
case "$1" in
  -s) echo "$FAKE_SYSTEM" ;;
  -m) echo "$FAKE_MACHINE" ;;
  *) exit 1 ;;
esac
"""
SW_VERS = """#!/bin/sh
[ "$1" = -productVersion ] && echo "$FAKE_MACOS"
"""


def checksum(data):
    return f"{hashlib.sha256(data).hexdigest()}  anywhere\n"


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        """The test's output is the script's, not each request's."""


class InstallTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.served = self.root / "served"
        self.builds = {}
        for target in TARGETS.values():
            folder = self.served / "cli" / target
            folder.mkdir(parents=True)
            build = BUILD.format(target=target).encode()
            (folder / "anywhere").write_bytes(build)
            (folder / "anywhere.sha256").write_text(checksum(build))
            self.builds[target] = build
        shutil.copyfile(SCRIPT, self.served / "install.sh")
        handler = functools.partial(Quiet, directory=str(self.served))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.base = f"http://127.0.0.1:{server.server_address[1]}"
        self.home = self.root / "home"
        self.home.mkdir()
        self.installed = self.home / ".local/bin/anywhere"
        self.calls = self.root / "calls"

    def tools(self, hashers=None):
        """A folder of the tools the script may use, with `hashers`, or whichever of the two are here."""
        folder = Path(tempfile.mkdtemp(dir=self.root))
        wanted = TOOLS + (hashers if hashers is not None else tuple(filter(shutil.which, HASHERS)))
        for tool in wanted:
            found = shutil.which(tool)
            if found is None:
                self.skipTest(f"{tool} isn't installed here")
            (folder / tool).symlink_to(found)
        for name, script in (("uname", UNAME), ("sw_vers", SW_VERS)):
            (folder / name).write_text(script)
            (folder / name).chmod(0o755)
        return folder

    def install(self, system="Darwin", machine="arm64", hashers=None, path=None, **settings):
        """Pipes the script, as served, into sh, as `curl ... | sh` does."""
        tools = self.tools(hashers)
        env = {"HOME": str(self.home), "PATH": f"{tools}{os.pathsep + path if path else ''}",
               "FAKE_SYSTEM": system, "FAKE_MACHINE": machine, "FAKE_MACOS": "26.2",
               "CALLS": str(self.calls), "ANYWHERE_INSTALL_BASE": self.base, **settings}
        return subprocess.run(["/bin/sh"], input=(self.served / "install.sh").read_bytes(), env=env,
                              capture_output=True, timeout=60, check=False)

    def assertInstalled(self, result, target):
        output = result.stdout.decode() + result.stderr.decode()
        self.assertEqual(result.returncode, 0, output)
        self.assertEqual(self.installed.read_bytes(), self.builds[target])
        self.assertEqual(self.installed.stat().st_mode & 0o777, 0o755)
        self.assertEqual(self.calls.read_text(), "--version\nskill install\n")
        self.assertIn(f"installed anywhere 1.2.3 ({target}) as {self.installed}", output)
        self.assertIn("installed the skill", output)
        self.assertEqual(list(self.installed.parent.iterdir()), [self.installed], "a temporary file was left")
        return output

    def assertRefused(self, result, message):
        output = result.stdout.decode() + result.stderr.decode()
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn(message, result.stderr.decode())
        folder = self.installed.parent
        if folder.exists():
            left = [path.name for path in folder.iterdir() if path.name != "anywhere"]
            self.assertEqual(left, [], "a temporary file was left")
        return output

    def test_it_installs_the_build_for_each_computer_and_then_the_skill(self):
        for (system, machine), target in TARGETS.items():
            with self.subTest(target=target):
                self.calls.unlink(missing_ok=True)
                output = self.assertInstalled(self.install(system, machine), target)
                # ~/.local/bin isn't on this PATH, so it says how to put it there.
                self.assertIn(f'export PATH="{self.installed.parent}:$PATH"', output)
                self.assertIn(f"next, sign in: {self.installed} login", output)

    def test_on_path_the_next_step_names_the_command_alone(self):
        result = self.install(path=str(self.installed.parent))
        output = self.assertInstalled(result, "aarch64-apple-darwin")
        self.assertNotIn("export PATH", output)
        self.assertNotIn("warning", output)
        self.assertIn("next, sign in: anywhere login", output)

    def test_another_anywhere_first_on_path_is_named_and_the_next_step_skips_it(self):
        other = self.root / "other"
        other.mkdir()
        (other / "anywhere").write_text("#!/bin/sh\necho an older CLI\n")
        (other / "anywhere").chmod(0o755)
        for path, hint in ((f"{other}{os.pathsep}{self.installed.parent}", False), (str(other), True)):
            with self.subTest(path=path):
                self.calls.unlink(missing_ok=True)
                output = self.assertInstalled(self.install(path=path), "aarch64-apple-darwin")
                self.assertIn(f"warning: another anywhere, {other}/anywhere, comes first on your PATH", output)
                self.assertEqual("export PATH" in output, hint)
                self.assertIn(f"next, sign in: {self.installed} login", output)

    def test_a_mac_older_than_the_cli_runs_on_is_refused_before_any_download(self):
        for version, refused in (("12.7.6", True), ("10.15.7", True), ("13.0", False), ("26.2", False)):
            with self.subTest(version=version):
                for machine, target in (("arm64", "aarch64-apple-darwin"), ("x86_64", "x86_64-apple-darwin")):
                    self.calls.unlink(missing_ok=True)
                    self.installed.unlink(missing_ok=True)
                    result = self.install("Darwin", machine, FAKE_MACOS=version)
                    if refused:
                        self.assertRefused(result, f"needs macOS 13 or later, and this Mac runs macOS {version}")
                        self.assertFalse(self.installed.exists())
                    else:
                        self.assertInstalled(result, target)
        self.assertRefused(self.install("Darwin", "arm64", FAKE_MACOS="beta"), "isn't a version")

    def test_either_hasher_checks_the_download_and_with_neither_nothing_installs(self):
        for hasher in HASHERS:
            with self.subTest(hasher=hasher):
                self.calls.unlink(missing_ok=True)
                self.assertInstalled(self.install(hashers=(hasher,)), "aarch64-apple-darwin")
        self.installed.unlink()
        self.assertRefused(self.install(hashers=()), "neither sha256sum nor shasum")
        self.assertFalse(self.installed.exists())

    def test_it_installs_into_the_folder_asked_and_replaces_a_cli_there(self):
        folder = self.root / "bin"
        folder.mkdir()
        (folder / "anywhere").write_text("an older CLI")
        result = self.install(ANYWHERE_INSTALL_DIR=str(folder))
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual((folder / "anywhere").read_bytes(), self.builds["aarch64-apple-darwin"])
        self.assertEqual([path.name for path in folder.iterdir()], ["anywhere"])
        self.assertFalse(self.installed.exists())

    def test_a_computer_it_has_no_build_for_is_refused_before_any_download(self):
        for system, machine in (("Linux", "aarch64"), ("FreeBSD", "amd64"), ("Darwin", "i386")):
            with self.subTest(system=system, machine=machine):
                result = self.install(system, machine)
                self.assertRefused(result, f"there's no anywhere CLI for {system} on {machine}")
                self.assertFalse(self.installed.exists())

    def test_a_download_that_doesnt_match_its_checksum_changes_nothing(self):
        self.installed.parent.mkdir(parents=True)
        self.installed.write_text("the CLI installed before")
        build = self.served / "cli/aarch64-apple-darwin/anywhere"
        build.write_bytes(build.read_bytes() + b"# changed on the way\n")
        self.assertRefused(self.install(), "nothing was installed")
        self.assertEqual(self.installed.read_text(), "the CLI installed before")
        self.assertFalse(self.calls.exists(), "the download ran")

    def test_a_checksum_file_without_its_last_newline_still_checks(self):
        sums = self.served / "cli/aarch64-apple-darwin/anywhere.sha256"
        sums.write_text(sums.read_text().rstrip("\n"))
        self.assertInstalled(self.install(), "aarch64-apple-darwin")

    def test_a_checksum_file_that_isnt_one_is_refused(self):
        sums = self.served / "cli/aarch64-apple-darwin/anywhere.sha256"
        good = sums.read_text()
        for bad in ("", "\n", good.upper(), good.replace("  anywhere", "  other"), good.replace("  ", " "),
                    good + good, good[1:], "z" + good[1:], good.rstrip("\n").replace("anywhere", "anywhere x\n")):
            with self.subTest(bad=bad):
                sums.write_text(bad)
                self.assertRefused(self.install(), str(self.base))
                self.assertFalse(self.installed.exists())

    def test_a_target_the_platform_lacks_is_refused(self):
        (self.served / "cli/x86_64-apple-darwin/anywhere").unlink()
        self.assertRefused(self.install("Darwin", "x86_64"), "isn't there (HTTP 404)")
        (self.served / "cli/aarch64-apple-darwin/anywhere.sha256").unlink()
        self.assertRefused(self.install(), "anywhere.sha256 isn't there (HTTP 404)")
        self.assertFalse(self.installed.exists())

    def test_a_platform_that_cant_be_reached_is_refused(self):
        # Nothing listens on port 9, the discard port, here.
        self.assertRefused(self.install(ANYWHERE_INSTALL_BASE="http://127.0.0.1:9"), "couldn't download")

    def test_only_an_https_origin_or_this_machine_is_a_base(self):
        for base in ("http://anywhere.so", "https://anywhere.so/", "https://anywhere.so/cli", "anywhere.so",
                     "https://", "https://anywhere.so?x", "https://user@anywhere.so", "ftp://anywhere.so",
                     "http://127.0.0.1.example.com", "https://anywhere.so evil"):
            with self.subTest(base=base):
                self.assertRefused(self.install(ANYWHERE_INSTALL_BASE=base), "ANYWHERE_INSTALL_BASE")

    def test_an_install_folder_must_be_absolute(self):
        self.assertRefused(self.install(ANYWHERE_INSTALL_DIR="bin"), "must be an absolute path")

    def test_a_build_that_doesnt_run_here_isnt_installed(self):
        build = b"#!/bin/sh\necho 'cannot execute binary file' >&2\nexit 126\n"
        (self.served / "cli/aarch64-apple-darwin/anywhere").write_bytes(build)
        (self.served / "cli/aarch64-apple-darwin/anywhere.sha256").write_text(checksum(build))
        self.assertRefused(self.install(), "doesn't run on this computer")
        self.assertFalse(self.installed.exists())

    def test_a_skill_install_that_fails_fails_the_install_and_keeps_the_cli(self):
        # 1 for an error, 2 for an argument error, which the CLI's own message explains.
        for status in ("1", "2"):
            with self.subTest(status=status):
                result = self.install(SKILL=status)
                self.assertRefused(result, "`anywhere skill install` failed, as it says above")
                self.assertIn(f"run `{self.installed} skill install` again", result.stderr.decode())
                self.assertNotIn(b"next, sign in", result.stdout)
                self.assertEqual(self.installed.read_bytes(), self.builds["aarch64-apple-darwin"])

    def test_a_cut_off_script_runs_nothing(self):
        script = (self.served / "install.sh").read_bytes()
        cut = script[: script.rindex(b"main ")]
        tools = self.tools()
        result = subprocess.run(["/bin/sh"], input=cut, env={"HOME": str(self.home), "PATH": str(tools),
                                "FAKE_SYSTEM": "Darwin", "FAKE_MACHINE": "arm64", "ANYWHERE_INSTALL_BASE": self.base},
                                capture_output=True, timeout=60, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(result.stdout + result.stderr, b"")
        self.assertFalse(self.installed.parent.exists())


if __name__ == "__main__":
    unittest.main()
