#!/usr/bin/env python3
"""Check expiry and configuration consistency for the maintenance exception."""
from datetime import date
from pathlib import Path
import tempfile
import unittest

from check_advisory_exceptions import ADVISORY, check


class AdvisoryExceptions(unittest.TestCase):
    def verify(self, audit, deny, today):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".cargo").mkdir()
            for name, ids in ((".cargo/audit.toml", audit), ("deny.toml", deny)):
                (root / name).write_text(f"[advisories]\nignore = {ids!r}\n")
            check(root, today)

    def test_reviewed_exception_before_deadline(self):
        self.verify([ADVISORY], [ADVISORY], date(2026, 12, 16))

    def test_deadline_and_later_fail(self):
        for today in (date(2026, 12, 17), date(2027, 1, 1)):
            with self.subTest(today=today), self.assertRaisesRegex(ValueError, "expired"):
                self.verify([ADVISORY], [ADVISORY], today)

    def test_inconsistent_or_broadened_exceptions_fail(self):
        for audit, deny in (([], [ADVISORY]), ([ADVISORY], []),
                            ([ADVISORY, "RUSTSEC-2000-0000"], [ADVISORY]),
                            (["RUSTSEC-2000-0000"], ["RUSTSEC-2000-0000"])):
            with self.subTest(audit=audit, deny=deny), self.assertRaises(ValueError):
                self.verify(audit, deny, date(2026, 11, 17))

    def test_removed_exceptions_pass_after_deadline(self):
        self.verify([], [], date(2027, 1, 1))


if __name__ == "__main__":
    unittest.main()
