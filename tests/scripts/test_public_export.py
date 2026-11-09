"""Regression checks for private-directory exclusion in public exports."""

import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


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

    def test_scrub_rejects_symlink_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            sentinel = output / "sentinel.txt"
            sentinel.write_text("keep\n")
            (output / "link.md").symlink_to(sentinel)
            with self.assertRaises(SystemExit):
                public_export.scrub(output)
            self.assertEqual(sentinel.read_text(), "keep\n")

    def test_export_excludes_archive_and_baselines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            for directory in ("archive", "baselines"):
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
            self.assertFalse((output / "docs/archive").exists())
            self.assertFalse((output / "docs/baselines").exists())

    def test_gate_rejects_archive_reintroduced_after_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "Cargo.toml").write_text(
                'include = [\n    "src/**",\n]\n'
            )
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(public_export.gate(output), 0)
            archive = output / "docs/archive"
            archive.mkdir(parents=True)
            (archive / "private.md").write_text("internal report\n")
            captured = io.StringIO()
            with contextlib.redirect_stdout(captured):
                self.assertEqual(public_export.gate(output), 1)
            self.assertIn("forbidden path present: docs/archive", captured.getvalue())

    def test_gate_rejects_private_export_scripts(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "Cargo.toml").write_text(
                'include = [\n    "src/**",\n]\n'
            )
            scripts = output / "tests/scripts"
            scripts.mkdir(parents=True)
            (scripts / "import_recall.py").write_text("# private corpus harness\n")
            captured = io.StringIO()
            with contextlib.redirect_stdout(captured):
                self.assertEqual(public_export.gate(output), 1)
            self.assertIn(
                "private script present: tests/scripts/import_recall.py",
                captured.getvalue(),
            )


    def _source_repo(self, root: Path) -> Path:
        source = root / "source"
        source.mkdir()
        (source / "README.md").write_text("public guide\n")
        subprocess.run(["git", "init", "-q", str(source)], check=True)
        subprocess.run(["git", "add", "."], cwd=source, check=True)
        subprocess.run(
            ["git", "-c", "user.name=Export Test", "-c",
             "user.email=export@example.invalid", "-c", "commit.gpgsign=false",
             "commit", "-qm", "fixture"],
            cwd=source, check=True,
        )
        return source

    def test_export_rejects_nonempty_destination_before_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self._source_repo(root)
            out = root / "export"
            out.mkdir()
            sentinel = out / "sentinel.txt"
            sentinel.write_text("keep\n")
            with self.assertRaises(SystemExit):
                public_export.export(source, out)
            self.assertEqual(sentinel.read_text(), "keep\n")
            self.assertFalse((out / "README.md").exists())

    def test_export_rejects_overlapping_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self._source_repo(root)
            with self.assertRaises(SystemExit):
                public_export.export(source, source / "export")

    def test_gate_fails_closed_on_scanner_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "Cargo.toml").write_text(
                'include = [\n    "src/**",\n]\n'
            )
            failed = subprocess.CompletedProcess(
                args=["git", "grep"], returncode=128, stdout="", stderr="scanner failed"
            )
            captured = io.StringIO()
            with mock.patch.object(public_export.subprocess, "run", return_value=failed):
                with contextlib.redirect_stdout(captured):
                    self.assertEqual(public_export.gate(output), 1)
            self.assertIn("scan-error 128", captured.getvalue())


    def test_invalid_source_does_not_remove_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "README.md").write_text("not a git repository\n")
            out = root / "export"
            out.mkdir()
            sentinel = out / "sentinel.txt"
            sentinel.write_text("keep\n")
            with self.assertRaises(subprocess.CalledProcessError):
                public_export.export(source, out)
            self.assertEqual(sentinel.read_text(), "keep\n")

    def test_allowlist_checker_fails_closed(self):
        script = Path(__file__).with_name("allowlist_check.sh")
        missing = subprocess.run(
            ["bash", str(script), "/definitely/missing.lock"],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("cannot read input", missing.stderr)

        with tempfile.TemporaryDirectory() as temporary:
            lock = Path(temporary) / "Cargo.lock"
            lock.write_text('[[package]]\nname = "definitely-not-allowed"\n')
            unlisted = subprocess.run(
                ["bash", str(script), str(lock)],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(unlisted.returncode, 0)
            self.assertIn("definitely-not-allowed", unlisted.stderr)


if __name__ == "__main__":
    unittest.main()
