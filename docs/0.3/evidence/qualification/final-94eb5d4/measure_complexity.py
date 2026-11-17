"""Supplement syntax metrics with physical/body lines and approximate complexity."""
import argparse,hashlib,json,subprocess,sys
from importlib.metadata import version
from pathlib import Path
from tree_sitter import Language,Parser
import tree_sitter_rust
assert sys.version_info[:2]==(3,13)
assert version('tree-sitter')=='0.25.2' and version('tree-sitter-rust')=='0.24.2'
cli=argparse.ArgumentParser();cli.add_argument('--output',type=Path,required=True);args=cli.parse_args()
engine=Parser(Language(tree_sitter_rust.language()));sha=lambda b:hashlib.sha256(b).hexdigest()
comments={'line_comment','block_comment'};testattrs={b'#[cfg(test)]',b'#[test]',b'#[tokio::test]'}

def analyze(data):
 tree=engine.parse(data);assert not tree.root_node.has_error
 excluded=set();bodylines=set();functions=[]
 def children(node):
  pending=False
  for child in node.children:
   if child.type=='attribute_item':
    if b''.join(data[child.start_byte:child.end_byte].split()) in testattrs:
     pending=True;excluded.update(range(child.start_point.row,child.end_point.row+1))
    continue
   if child.type in comments:continue
   if pending:
    pending=False;excluded.update(range(child.start_point.row,child.end_point.row+1));continue
   yield child
 def body(node,lines):
  if node.type in comments:return 0,0
  complexity=0;macros=int(node.type=='macro_invocation')
  if node.type in {'if_expression','while_expression','for_expression','try_expression'}:complexity+=1
  if node.type=='match_expression':
   block=node.child_by_field_name('body');complexity+=max(0,sum(c.type=='match_arm' for c in block.children)-1)
  if node.type=='binary_expression':
   operator=node.child_by_field_name('operator')
   if operator and data[operator.start_byte:operator.end_byte] in {b'&&',b'||'}:complexity+=1
  if not node.children:lines.update(range(node.start_point.row,node.end_point.row+1))
  for child in children(node):
   c,m=body(child,lines);complexity+=c;macros+=m
  return complexity,macros
 def visit(node):
  if node.type=='function_item':
   block=node.child_by_field_name('body');lines=set();decisions,macros=body(block,lines);bodylines.update(lines)
   name=node.child_by_field_name('name');functions.append(dict(name=data[name.start_byte:name.end_byte].decode(),line=node.start_point.row+1,body_token_lines=len(lines),approximate_cyclomatic=1+decisions,unexpanded_macro_invocations=macros));return
  for child in children(node):visit(child)
 visit(tree.root_node)
 return dict(source_sha256=sha(data),physical_file_lines=len(data.splitlines()),physical_lines_excluding_exact_test_items=len(set(range(len(data.splitlines())))-excluded),function_body_token_lines=len(bodylines),functions=functions)
lanes={}
for label,ref in [('original','c418e98'),('candidate','e880a2a')]:
 rev=subprocess.check_output(['git','rev-parse',ref],text=True).strip();paths=subprocess.check_output(['git','ls-tree','-r','--name-only',rev],text=True).splitlines()
 files={p:analyze(subprocess.check_output(['git','show',rev+':'+p])) for p in paths if p.startswith('crates/') and '/src/' in p and p.endswith('.rs')}
 totals={k:sum(f[k] for f in files.values()) for k in ['physical_file_lines','physical_lines_excluding_exact_test_items','function_body_token_lines']}
 totals.update(function_count=sum(len(f['functions']) for f in files.values()),approximate_cyclomatic_sum=sum(v['approximate_cyclomatic'] for f in files.values() for v in f['functions']))
 lanes[label]=dict(revision=rev,files=files,totals=totals)
result=dict(runner_sha256=sha(Path(__file__).read_bytes()),parser={'python':sys.version,'tree-sitter':version('tree-sitter'),'tree-sitter-rust':version('tree-sitter-rust')},methodology='Per-function approximation: 1 + if/while/for/try + short-circuit boolean operators + max(match arm count - 1, 0). Body token lines include braces and declarations within bodies; they approximate executable source, not machine instructions.',limitations=['Physical full-file lines include comments, blanks and tests; a second count excludes exact test-attributed item/attribute rows.','Same exact cfg(test)/test exclusions as source metrics; complex cfg conditions remain. Mutually exclusive branches may both count.','Macros are not expanded. Their invocation count is reported; generated decision paths cannot be inferred from source tokens.','Unconditional loop alone adds no decision. Panic/unwind and implicit library/control-flow behavior are not modeled.','Sum includes one base per function and is not a whole-program path count. Nested function syntax, when present, is charged to its enclosing body.'],lanes=lanes)
args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v['totals'] for k,v in lanes.items()},indent=2))
