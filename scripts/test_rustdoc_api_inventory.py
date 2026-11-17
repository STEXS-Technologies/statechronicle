#!/usr/bin/env python3
"""Compiler-backed checks for API review inventory; requires installed nightly."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from rustdoc_api_inventory import InventoryError, inventory

SOURCE = '''
#![crate_name="api_fixture"]
mod hidden { pub struct Hidden { pub exposed: u32 } }
pub use hidden::Hidden as Renamed;
#[repr(C)]
#[non_exhaustive]
#[must_use]
pub struct Record { pub value: u32 }
#[doc(hidden)] pub struct HiddenDocumented;
pub enum Ordered { First, Second }
macro_rules! impl_marker { ($t:ty) => { impl Marker for $t { fn defaulted(&self) {} } }; }
pub trait Marker { fn defaulted(&self) {} }
impl_marker!(Record);
pub fn bounded<T: Marker>(value: T) -> T { value }
'''


class CompilerInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='statechronicle-api-test-')
        cls.root = Path(cls.temp.name)
        cls.document = cls.compile(SOURCE, 'base')
        cls.baseline = inventory(cls.document)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    @classmethod
    def compile(cls, source, lane):
        root = cls.root / lane
        root.mkdir()
        path = root / 'api.rs'
        path.write_text(source)
        subprocess.run(['rustdoc', '+nightly', str(path), '--edition', '2024',
                        '-Z', 'unstable-options', '--output-format', 'json',
                        '--document-hidden-items', '-o', str(root / 'doc')],
                       check=True, capture_output=True, text=True)
        return json.loads((root / 'doc/api_fixture.json').read_text())

    def test_hidden_public_alias_and_expanded_impl_contracts(self):
        exports = self.baseline['exports']
        self.assertNotIn('api_fixture::hidden', exports)
        self.assertIn('api_fixture::HiddenDocumented', exports)
        alias = exports['api_fixture::Renamed']['local_target']
        self.assertIn('exposed', json.dumps(alias))
        record = exports['api_fixture::Record']
        self.assertEqual(len(record['attributes']), 3)
        impls = record['declaration']['struct']['impls']
        self.assertTrue(any(i['declaration']['impl']['trait'] is not None
                            and i['declaration']['impl']['trait']['definition']['path']
                            == 'api_fixture::Marker' for i in impls))

    def test_compiler_ids_and_unordered_impl_order_do_not_change_inventory(self):
        document = copy.deepcopy(self.document)
        # Rewrite reference positions only, never discriminants, lengths or values.
        def rewrite(value, key=None):
            if key in {'id', 'root'} and type(value) is int:
                return value + 100000
            if key in {'items', 'impls', 'implementations', 'fields', 'variants', 'tuple'} and isinstance(value, list) and all(x is None or type(x) is int for x in value):
                return [None if x is None else x + 100000 for x in value]
            if isinstance(value, dict):
                return {k: rewrite(v, k) for k, v in value.items()}
            if isinstance(value, list):
                return [rewrite(v) for v in value]
            return value
        document = rewrite(document)
        for field in ['index', 'paths']:
            document[field] = {str(int(k) + 100000): v for k, v in document[field].items()}
        for value in document['index'].values():
            if 'struct' in value['inner']:
                value['inner']['struct']['impls'].reverse()
        self.assertEqual(inventory(document), self.baseline)

    def test_real_compiler_contract_changes_are_detected(self):
        mutations = [
            ('field', 'pub value: u32', 'pub value: u64', 'Record'),
            ('alias', 'pub exposed: u32', 'pub exposed: u64', 'Renamed'),
            ('order', 'First, Second', 'Second, First', 'Ordered'),
            ('bound', 'T: Marker', 'T: Marker + Send', 'bounded'),
            ('default', 'pub trait Marker { fn defaulted(&self) {} }', 'pub trait Marker { fn defaulted(&self); }', 'Marker'),
            ('repr', '#[repr(C)]', '#[repr(transparent)]', 'Record'),
        ]
        for lane, before, after, name in mutations:
            with self.subTest(lane=lane):
                changed = inventory(self.compile(SOURCE.replace(before, after), lane))
                key = 'api_fixture::' + name
                self.assertNotEqual(self.baseline['exports'][key], changed['exports'][key])

    def test_removed_export_is_detected_after_successful_compilation(self):
        changed = inventory(self.compile(SOURCE.replace(
            '#[doc(hidden)] pub struct HiddenDocumented;',
            '#[doc(hidden)] struct HiddenDocumented;'), 'removed'))
        self.assertEqual(set(self.baseline['exports']) - set(changed['exports']),
                         {'api_fixture::HiddenDocumented'})

    def test_unsupported_or_private_collection_is_rejected(self):
        for field, value in [('format_version', 60), ('includes_private', True)]:
            document = copy.deepcopy(self.document)
            document[field] = value
            with self.assertRaises(InventoryError):
                inventory(document)


if __name__ == '__main__':
    unittest.main()
