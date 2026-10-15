import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class ContainerCommands(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.script = Path(__file__).with_name("container.sh").resolve()
        self.worktree = self.script.parents[2]
        self.log = self.directory / "calls.jsonl"
        docker = self.directory / "docker"
        docker.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "with open(os.environ['MOCK_LOG'], 'a') as log:\n"
            "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "if sys.argv[1] == 'inspect':\n"
            "    print(os.environ['MOCK_OWNER'])\n"
            "if sys.argv[-1] == '/run/dbus/system_bus_socket' and os.environ.get('MOCK_BOOT_ERROR'):\n"
            "    sys.exit(1)\n"
        )
        docker.chmod(0o755)
        sleep = self.directory / "sleep"
        sleep.write_text("#!/bin/sh\nexit 0\n")
        sleep.chmod(0o755)
        self.environment = dict(os.environ)
        self.environment.update(
            PATH=f"{self.directory}:{os.environ['PATH']}",
            MOCK_LOG=str(self.log),
            MOCK_OWNER=str(self.worktree),
        )

    def run_script(self, *arguments):
        return subprocess.run(
            ["bash", str(self.script), *arguments],
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
            timeout=10,
        )

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_rejects_other_worktrees_before_exec_or_removal(self):
        self.environment["MOCK_OWNER"] = "/another/worktree"
        for arguments in [("exec", "fixture", "true"), ("down", "fixture")]:
            with self.subTest(arguments=arguments):
                result = self.run_script(*arguments)
                self.assertNotEqual(result.returncode, 0)
        self.assertTrue(all(call[0] == "inspect" for call in self.calls()))

    def test_exec_preserves_argument_boundaries_as_the_test_user(self):
        result = self.run_script("exec", "fixture", "printf", "one two", "--literal")
        self.assertEqual(result.returncode, 0, result.stderr)
        command = self.calls()[-1]
        self.assertEqual(command[-4:], ["pv-linux-fixture", "printf", "one two", "--literal"])
        self.assertEqual(command[1:3], ["--user", "dev"])
        self.assertIn("HOME=/home/dev", command)

    def test_down_stops_then_removes_the_owned_container(self):
        result = self.run_script("down", "fixture")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([call[0] for call in calls], ["inspect", "stop", "rm"])
        self.assertTrue(all(call[-1] == "pv-linux-fixture" for call in calls))

    def test_up_marks_ownership_and_mounts_only_the_readonly_worktree(self):
        result = self.run_script("up", "fixture", "test-image")
        self.assertEqual(result.returncode, 0, result.stderr)
        command = self.calls()[0]
        self.assertIn(f"dev.pv.linux-worktree={self.worktree}", command)
        self.assertIn(f"type=bind,src={self.worktree},dst=/src,readonly", command)
        self.assertEqual(command[-2:], ["test-image", "/sbin/init"])

    def test_invalid_names_never_reach_docker(self):
        for name in ["", "../peer", "UPPER"]:
            with self.subTest(name=name):
                self.assertEqual(self.run_script("down", name).returncode, 2)
        self.assertEqual(self.calls(), [])

    def test_up_fails_when_the_system_bus_never_becomes_ready(self):
        self.environment["MOCK_BOOT_ERROR"] = "1"
        result = self.run_script("up", "fixture", "test-image")
        self.assertEqual(result.returncode, 1)
        self.assertFalse(any("loginctl" in call for call in self.calls()))


if __name__ == "__main__":
    unittest.main()
