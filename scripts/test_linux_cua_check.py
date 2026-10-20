import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("linux_cua_check.py")


class LinuxCuaCheckTests(unittest.TestCase):
    def run_check(self, config: Path, *, display: bool) -> tuple[int, dict]:
        environment = os.environ.copy()
        if not display:
            environment.pop("DISPLAY", None)
            environment.pop("WAYLAND_DISPLAY", None)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--config", str(config)],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )
        return result.returncode, json.loads(result.stdout)

    def test_missing_config_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            code, result = self.run_check(Path(directory) / "missing.toml", display=True)
        self.assertEqual(code, 2)
        self.assertEqual(result["status"], "error")

    def test_missing_graphical_session_has_distinct_code(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.toml"
            config.write_text(
                '[mcp_servers.linux_cua]\ncommand = "/bin/true"\n',
                encoding="utf-8",
            )
            code, result = self.run_check(config, display=False)
        self.assertEqual(code, 3)
        self.assertIn("DISPLAY", result["message"])

    def test_invalid_toml_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.toml"
            config.write_text("not = [valid", encoding="utf-8")
            code, result = self.run_check(config, display=True)
        self.assertEqual(code, 2)
        self.assertEqual(result["status"], "error")


if __name__ == "__main__":
    unittest.main()
