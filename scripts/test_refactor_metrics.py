"""Guard the scope of measurements used as release evidence."""
import unittest
from tree_sitter import Language, Parser
import tree_sitter_rust
from refactor_metrics import analyze


class MeasurementScope(unittest.TestCase):
    def setUp(self):
        self.language = Language(tree_sitter_rust.language())
        self.parser = Parser(self.language)

    def measure(self, text):
        return analyze(text.encode(), 'fixture.rs', self.parser)

    def test_test_module_and_comments_do_not_inflate_production(self):
        plain = self.measure('pub fn run() { if true {} }')
        decorated = self.measure('''// commentary
pub fn run() { if true {} }
#[cfg(test)]
mod tests { pub fn expensive_test() { for i in 0..100 { if i > 1 {} } } }
''')
        for key in ('lexical_tokens', 'branch_nodes', 'public_syntax_items'):
            self.assertEqual(plain[key], decorated[key])
        self.assertEqual([f['name'] for f in decorated['functions']], ['run'])

    def test_production_cfg_is_retained(self):
        item = self.measure('#[cfg(not(test))] pub fn production() { if true {} }')
        self.assertEqual(item['branch_nodes'], 1)
        self.assertEqual(item['public_syntax_items'], 1)

    def test_public_trait_contract_keeps_associated_signatures(self):
        item = self.measure('pub trait Model { type State; fn apply(&self); }')
        self.assertEqual(item['public_syntax_items'], 3)

    def test_malformed_rust_cannot_silently_reduce_totals(self):
        with self.assertRaises(ValueError):
            self.measure('pub fn broken(')


if __name__ == '__main__':
    unittest.main()
