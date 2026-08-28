import subprocess
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[2]
RESOLVER = ROOT / "scripts" / "resolve_rusty_v8_artifacts.sh"


class RustyV8ResolverTest(unittest.TestCase):
    def test_local_artifacts_are_exported_to_child_process(self) -> None:
        with TemporaryDirectory() as temp_dir:
            local_repo = Path(temp_dir)
            archive = local_repo / (
                "librusty_v8_ptrcomp_sandbox_release_"
                "aarch64-linux-android.a.gz"
            )
            binding = local_repo / (
                "src_binding_ptrcomp_sandbox_release_"
                "aarch64-linux-android.rs"
            )
            archive.write_bytes(b"fixture archive")
            binding.write_text("fixture binding\n", encoding="utf-8")

            resolver = subprocess.run(
                [
                    "bash",
                    str(RESOLVER),
                    "--target=aarch64-linux-android",
                    "--output-dir=" + str(local_repo / "output"),
                    "--local-repo=" + str(local_repo),
                    "--v8-version=150.4.0",
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            probe = subprocess.run(
                [
                    "bash",
                    "-c",
                    (
                        'eval "$1"; '
                        'test "$RUSTY_V8_ARCHIVE" = "$2"; '
                        'test "$RUSTY_V8_SRC_BINDING_PATH" = "$3"'
                    ),
                    "rusty-v8-env-probe",
                    resolver.stdout,
                    str(archive),
                    str(binding),
                ],
                check=False,
            )

            self.assertEqual(probe.returncode, 0)


if __name__ == "__main__":
    unittest.main()
