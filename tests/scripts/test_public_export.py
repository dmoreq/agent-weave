"""Regression checks for private-directory exclusion in public exports."""

import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "public_export", Path(__file__).with_name("public_export.py")
)
public_export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(public_export)


class PublicExportTests(unittest.TestCase):
    def test_scrub_excludes_exporter_and_its_private_tests(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            scripts = output / "tests/scripts"
            scripts.mkdir(parents=True)
            for name in ("public_export.py", "test_public_export.py"):
                (scripts / name).write_text("# private export tooling\n")
            public_export.scrub(output)
            self.assertFalse((scripts / "public_export.py").exists())
            self.assertFalse((scripts / "test_public_export.py").exists())

    def test_export_excludes_reports_and_baselines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            for directory in ("reports", "baselines"):
                path = source / "docs" / directory
                path.mkdir(parents=True)
                (path / "private.md").write_text("internal evidence\n")
            (source / "README.md").write_text("public guide\n")
            subprocess.run(["git", "init", "-q", str(source)], check=True)
            subprocess.run(["git", "add", "."], cwd=source, check=True)
            subprocess.run(
                ["git", "-c", "user.name=Export Test", "-c",
                 "user.email=export@example.invalid", "-c", "commit.gpgsign=false",
                 "commit", "-qm", "fixture"],
                cwd=source, check=True,
            )
            output = root / "export"
            public_export.export(source, output)
            self.assertTrue((output / "README.md").is_file())
            self.assertFalse((output / "docs/reports").exists())
            self.assertFalse((output / "docs/baselines").exists())

    def test_gate_rejects_reports_reintroduced_after_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "Cargo.toml").write_text(
                'exclude = ["docs/baselines/**"]\n'
            )
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(public_export.gate(output), 0)
            reports = output / "docs/reports"
            reports.mkdir(parents=True)
            (reports / "private.md").write_text("internal report\n")
            captured = io.StringIO()
            with contextlib.redirect_stdout(captured):
                self.assertEqual(public_export.gate(output), 1)
            self.assertIn("forbidden path present: docs/reports", captured.getvalue())


if __name__ == "__main__":
    unittest.main()
