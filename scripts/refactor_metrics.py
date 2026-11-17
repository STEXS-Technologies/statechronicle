#!/usr/bin/env python3
"""Measure Rust syntax consistently; requires tree-sitter and tree-sitter-rust.

Excludes comments and items behind exact cfg(test)/test attributes. Counts all
other conditional code, including mutually exclusive feature/target branches.
Public syntax is not a reachability-resolved Rust API or cyclomatic analysis.
"""
import argparse
from collections import defaultdict
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import subprocess
import sys

from tree_sitter import Language, Parser
import tree_sitter_rust


COMMENT = {'line_comment', 'block_comment'}
BRANCH = {'if_expression', 'match_arm', 'while_expression', 'for_expression',
          'loop_expression', 'try_expression'}


def analyze(data, path, parser):
    tree = parser.parse(data)
    if tree.root_node.has_error:
        raise ValueError(f'Parse errors in {path}')
    tokens = []
    public = []
    functions = []
    branches = 0

    def walk(node, in_trait=False):
        nonlocal branches
        if node.type in COMMENT:
            return
        if node.type in BRANCH:
            branches += 1
        visible = any(c.type == 'visibility_modifier' for c in node.children)
        if visible or (in_trait and node.type in {'function_signature_item',
                                                  'function_item', 'associated_type',
                                                  'const_item'}):
            body = node.child_by_field_name('body')
            end = body.start_byte if body else node.end_byte
            public.append({'kind': node.type, 'line': node.start_point.row + 1,
                           'declaration': data[node.start_byte:end].decode().strip()})
        before = len(tokens)
        if not node.children:
            if node.type not in COMMENT:
                tokens.append(node)
            return
        pending_test = False
        for child in node.children:
            if child.type == 'attribute_item':
                text = b''.join(data[child.start_byte:child.end_byte].split())
                if text in {b'#[cfg(test)]', b'#[test]', b'#[tokio::test]'}:
                    pending_test = True
                # Attribute syntax itself is metadata, not executable syntax.
                continue
            if child.type in COMMENT:
                continue
            if pending_test:
                pending_test = False
                continue
            walk(child, in_trait or node.type == 'trait_item')
        if node.type == 'function_item':
            leaves = tokens[before:]
            normalized = b'\0'.join(data[t.start_byte:t.end_byte] for t in leaves)
            name = node.child_by_field_name('name')
            functions.append({'name': data[name.start_byte:name.end_byte].decode(),
                              'line': node.start_point.row + 1,
                              'tokens': len(leaves),
                              'sha256': hashlib.sha256(normalized).hexdigest()})

    walk(tree.root_node)
    lines = set()
    for token in tokens:
        lines.update(range(token.start_point.row + 1, token.end_point.row + 2))
    return {'source_sha256': hashlib.sha256(data).hexdigest(),
            'tracked_bytes': len(data), 'syntax_lines': len(lines),
            'lexical_tokens': len(tokens), 'branch_nodes': branches,
            'public_syntax_items': len(public), 'functions': functions,
            'public_declarations': public}


def main():
    if sys.version_info[:2] != (3, 13) or version('tree-sitter') != '0.25.2' or version('tree-sitter-rust') != '0.24.2':
        raise SystemExit('Use pinned Python 3.13, tree-sitter 0.25.2 and tree-sitter-rust 0.24.2 (see baseline.md)')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(['git', 'rev-parse', args.revision], cwd=root).decode().strip()
    paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', revision], cwd=root).decode().splitlines()
    rust = sorted(p for p in paths if p.startswith('crates/') and '/src/' in p and p.endswith('.rs'))
    language = Language(tree_sitter_rust.language())
    engine = Parser(language)
    files = {p: analyze(subprocess.check_output(['git', 'show', f'{revision}:{p}'], cwd=root), p, engine)
             for p in rust}
    groups = defaultdict(list)
    for path, item in files.items():
        for function in item['functions']:
            if function['tokens'] >= 30:
                groups[function['sha256']].append({'path': path, **function})
    result = {
        'revision': revision,
        'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'python': sys.version,
        'parser': {'tree-sitter': version('tree-sitter'),
                   'tree-sitter-rust': version('tree-sitter-rust')},
        'methodology': __doc__,
        'limitations': ['Syntax counts, not compiled code size or runtime branches.',
                        'Does not recognize complex test cfg expressions or expand macros.',
                        'Public syntax includes non-exported items and misses macro-generated API.',
                        'Duplicate groups detect exact token equality, not semantic duplication.'],
        'totals': {key: sum(f[key] for f in files.values()) for key in
                   ('tracked_bytes', 'syntax_lines', 'lexical_tokens', 'branch_nodes',
                    'public_syntax_items')},
        'files': files,
        'exact_duplicate_functions': [g for g in groups.values() if len(g) > 1],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result['totals']))


if __name__ == '__main__':
    main()
