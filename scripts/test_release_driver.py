#!/usr/bin/env python3
"""Exercise release-gate coverage and fail-closed orchestration without running tools."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

# Use system Python, independently of the fake python3 command on PATH.
FAKE = '''#!/usr/bin/python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['DRIVER_LOG'], 'a') as f:
    f.write(json.dumps([name, *args]) + '\\n')
if name == 'rustc':
    print('host: x86_64-unknown-linux-gnu')
if name == 'cargo' and args == ['+nightly', 'fuzz', 'list']:
    if os.environ.get('DRIVER_EMPTY') != '1':
        print('fuzz_first\\nfuzz_second')
if [name, *args] == json.loads(os.environ.get('DRIVER_FAIL', 'null')):
    sys.exit(29)
'''


class ReleaseDriver(unittest.TestCase):
    def execute(self, fail=None, empty=False):
        with tempfile.TemporaryDirectory(prefix='statechronicle-driver-') as temp:
            app = Path(temp)
            (app / 'scripts').mkdir()
            for name in ['run_release_checks.sh', 'check_embedded.sh']:
                shutil.copy(ROOT / 'scripts' / name, app / 'scripts' / name)
            commands = app / 'bin'
            commands.mkdir()
            for name in ['cargo', 'rustup', 'rustc', 'git', 'python3']:
                tool = commands / name
                tool.write_text(FAKE)
                tool.chmod(0o755)
            log = app / 'commands.jsonl'
            env = dict(os.environ, PATH=str(commands) + ':' + os.environ['PATH'],
                       DRIVER_LOG=str(log), DRIVER_FAIL=json.dumps(fail),
                       DRIVER_EMPTY='1' if empty else '0')
            result = subprocess.run(['bash', str(app / 'scripts/run_release_checks.sh')],
                                    env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            return result, calls

    def test_success_covers_docs_external_apps_and_all_active_audits(self):
        result, calls = self.execute()
        self.assertEqual(result.returncode, 0, result.stdout.decode())
        self.assertIn(b'release checks passed', result.stdout)
        self.assertEqual(calls[0], ['python3', 'scripts/check_advisory_exceptions.py'])
        self.assertIn(['python3', 'scripts/test_advisory_exceptions.py'], calls)
        self.assertIn(['cargo', 'test', '--workspace', '--exclude', 'statechronicle-fuzz',
                       '--doc', '--all-features', '--locked'], calls)
        for tutorial in ['minimal-native-domain', 'durable-custom-ledger']:
            consumer = ['python3', 'scripts/check_tutorials.py', '--tutorial', tutorial]
            self.assertIn(consumer, calls)
            self.assertLess(calls.index(['cargo', 'fetch', '--locked']), calls.index(consumer))
        self.assertIn(['python3', 'scripts/check_native_contracts.py'], calls)
        self.assertLess(calls.index(['cargo', 'fetch', '--locked']),
                        calls.index(['python3', 'scripts/check_native_contracts.py']))
        self.assertIn(['cargo', 'test', '--manifest-path', 'ci/signature-conformance/Cargo.toml',
                       '--release', '--locked'], calls)
        for lane in ['consumer', 'candidate']:
            self.assertIn(['cargo', 'test', '--manifest-path', f'ci/wire-0.3/{lane}/Cargo.toml',
                           '--all-targets', '--locked'], calls)
        for lock in ['Cargo.lock', 'ci/conformance/Cargo.lock',
                     'ci/signature-conformance/Cargo.lock', 'ci/wire-0.3/consumer/Cargo.lock',
                     'ci/wire-0.3/candidate/Cargo.lock',
                     'tutorials/minimal-native-domain/Cargo.lock',
                     'tutorials/durable-custom-ledger/Cargo.lock']:
            self.assertIn(['cargo', 'audit', '--file', lock, '--deny', 'warnings'], calls)
        self.assertIn(['rustup', 'toolchain', 'install', 'nightly', '--profile', 'minimal'], calls)
        runs = [c for c in calls if c[:4] == ['cargo', '+nightly', 'fuzz', 'run']]
        self.assertEqual([c[4] for c in runs], ['fuzz_first', 'fuzz_second'])
        for call in runs:
            self.assertIn('-runs=100', call)
            self.assertIn('-max_total_time=5', call)
        self.assertEqual(calls[-1], ['git', 'diff', '--check'])

    def test_every_external_failure_stops_before_success_or_later_commands(self):
        result, calls = self.execute()
        self.assertEqual(result.returncode, 0)
        # Each observed external boundary is failed independently, including
        # audit warning exit, tutorial output checks, toolchain setup and fuzz.
        for index, call in enumerate(calls):
            with self.subTest(command=call):
                result, observed = self.execute(fail=call)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn(b'release checks passed', result.stdout)
                self.assertEqual(observed, calls[:index + 1])

    def test_empty_manifest_cannot_report_fuzz_success(self):
        result, calls = self.execute(empty=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b'release checks passed', result.stdout)
        self.assertFalse(any(c[:4] == ['cargo', '+nightly', 'fuzz', 'run'] for c in calls))


if __name__ == '__main__':
    unittest.main()
