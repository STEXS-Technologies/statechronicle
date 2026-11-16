#!/usr/bin/env python3
"""Normalize public rustdoc JSON for review, not automatic SemVer certification.

Requires rustdoc JSON format 61. Removes compiler-local IDs, spans, documentation,
lint/codegen attributes and declaration order for unordered item/impl lists.
Retains ordered fields/variants, types, generics, bounds, ABI, visibility,
non-exhaustive/repr/must-use attributes, trait defaults and compiler-expanded impls.
Only the documented public module tree is exported; use --document-hidden-items
when collecting JSON so hidden public APIs are not omitted by rustdoc.
"""
import argparse
import json
from pathlib import Path


class InventoryError(ValueError):
    pass


def inventory(document):
    if document.get('format_version') != 61 or document.get('includes_private'):
        raise InventoryError('requires format 61 without private-item collection')
    index = {str(key): value for key, value in document['index'].items()}
    paths = {str(key): value for key, value in document['paths'].items()}
    active = set()

    def reference(identifier):
        key = str(identifier)
        if key in paths:
            return {'path': '::'.join(paths[key]['path']), 'kind': paths[key]['kind']}
        if key not in index:
            raise InventoryError(f'unresolved item ID {identifier}')
        if key in active:
            raise InventoryError(f'cyclic anonymous item ID {identifier}')
        active.add(key)
        value = item(index[key])
        active.remove(key)
        return {'item': value}

    def normalize(value, key=None):
        if isinstance(value, dict):
            # Resolve lexical aliases to the compiler's canonical definition path.
            if 'id' in value and 'path' in value and value['id'] is not None:
                result = {name: normalize(child, name) for name, child in value.items() if name not in {'id', 'path'}}
                result['definition'] = reference(value['id'])
                return result
            result = {}
            for name, child in value.items():
                if name in {'impls', 'implementations'}:
                    result[name] = sorted((item(index[str(identifier)]) for identifier in child), key=lambda x: json.dumps(x, sort_keys=True))
                elif name == 'items':
                    result[name] = sorted((item(index[str(identifier)]) for identifier in child), key=lambda x: json.dumps(x, sort_keys=True))
                elif name in {'fields', 'variants', 'tuple'} and isinstance(child, list) and all(x is None or type(x) is int for x in child):
                    result[name] = [None if identifier is None else item(index[str(identifier)]) for identifier in child]
                elif name == 'id':
                    result['definition'] = None if child is None else reference(child)
                else:
                    result[name] = normalize(child, name)
            return result
        if isinstance(value, list):
            return [normalize(child, key) for child in value]
        return value

    def item(value):
        attrs = []
        for attr in value.get('attrs', []):
            # Rustdoc represents semantic attributes structurally or as text.
            text = json.dumps(attr, sort_keys=True)
            if any(name in text for name in ['non_exhaustive', '"repr"', 'repr(', 'must_use']):
                # Lint configuration is not a public attribute contract.
                if 'LintCheck' not in text:
                    attrs.append(attr)
        return {'name': value['name'], 'visibility': value['visibility'], 'attributes': attrs,
                'deprecation': value.get('deprecation'), 'declaration': normalize(value['inner'])}

    exports = {}
    def visit(identifier, prefix):
        value = index[str(identifier)]
        if value['visibility'] != 'public':
            return
        inner = value['inner']
        name = value['name']
        if 'use' in inner:
            name = inner['use']['name']
            if inner['use']['is_glob']:
                name = '@glob:' + inner['use']['source']
        if name is None:
            raise InventoryError('public item has no export name')
        path = prefix + [name]
        export_path = '::'.join(path)
        if export_path in exports:
            raise InventoryError(f'duplicate export path {export_path}')
        if 'module' in inner:
            # Children have their own export paths; avoid duplicating whole subtrees.
            declaration = dict(value)
            declaration['inner'] = {'module': {name: child for name, child in inner['module'].items() if name != 'items'}}
            exports['::'.join(path)] = item(declaration)
            for child in inner['module']['items']:
                visit(child, path)
        else:
            record = item(value)
            if 'use' in inner:
                target = index.get(str(inner['use']['id']))
                if target is not None:
                    # An alias may expose a definition from a private module.
                    # Retain its contract even when the module is not exported.
                    record['local_target'] = item(target)
            exports[export_path] = record
    visit(document['root'], [])
    return {'format_version': 1, 'rustdoc_format': 61, 'crate_version': document.get('crate_version'),
            'exports': dict(sorted(exports.items()))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.write_text(json.dumps(inventory(json.loads(args.input.read_text())), indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    main()
