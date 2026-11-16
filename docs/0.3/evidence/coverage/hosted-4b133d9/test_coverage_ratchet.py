#!/usr/bin/env python3
"""Check coverage arithmetic and refusal of malformed evidence with real jq/awk."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().with_name('check_coverage_ratchet.sh')


def report(*lines):
    return {'data': [{'totals': {'lines': value}} for value in lines]}


class CoverageRatchet(unittest.TestCase):
    def check(self, evidence, minimum=92.4):
        with tempfile.TemporaryDirectory(prefix='statechronicle-coverage-') as temp:
            directory = Path(temp)
            (directory / 'report.json').write_text(json.dumps(evidence))
            (directory / 'baseline.json').write_text(json.dumps({'line_coverage_percent': minimum}))
            return subprocess.run(['bash', str(SCRIPT), str(directory / 'report.json'),
                                   str(directory / 'baseline.json')],
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    def test_weighted_counts_and_empty_translation_unit(self):
        result = self.check(report({'count': 100, 'covered': 94}, {'count': 0, 'covered': 0}))
        self.assertEqual(result.returncode, 0, result.stdout.decode())
        self.assertIn(b'94.0000%', result.stdout)
        # A mean of percentages would incorrectly pass this unequal-size pair.
        result = self.check(report({'count': 1, 'covered': 1}, {'count': 99, 'covered': 90}))
        self.assertNotEqual(result.returncode, 0)

    def test_existing_boundary_tolerance_and_regression(self):
        self.assertEqual(self.check(report({'count': 1000, 'covered': 924})).returncode, 0)
        self.assertNotEqual(self.check(report({'count': 1000, 'covered': 923})).returncode, 0)

    def test_malformed_totals_cannot_be_promoted_to_a_pass(self):
        malformed = [
            {'count': 100, 'covered': 101},
            {'count': -1, 'covered': 0},
            {'count': 0, 'covered': 1},
            {'count': 1.5, 'covered': 1.5},
            {'count': 100, 'covered': -1},
            {'count': '100', 'covered': 100},
            {'count': 100, 'covered': None},
            {'count': 100},
        ]
        for value in malformed:
            with self.subTest(lines=value):
                # A valid entry must not hide a malformed zero/negative entry.
                result = self.check(report({'count': 1000, 'covered': 1000}, value))
                self.assertNotEqual(result.returncode, 0, result.stdout.decode())
        for evidence in [{}, {'data': []}, report({'count': 0, 'covered': 0})]:
            with self.subTest(evidence=evidence):
                self.assertNotEqual(self.check(evidence).returncode, 0)

    def test_invalid_baseline_refuses_even_perfect_coverage(self):
        for minimum in [-1, 100.1, '92.4', None]:
            with self.subTest(minimum=minimum):
                self.assertNotEqual(self.check(report({'count': 100, 'covered': 100}), minimum).returncode, 0)


if __name__ == '__main__':
    unittest.main()
