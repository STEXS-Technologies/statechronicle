"""Measure the same native planning consumer before/after command ergonomics."""
import argparse,hashlib,importlib.util,json,re,subprocess,sys
from importlib.metadata import version
from pathlib import Path
from tree_sitter import Language,Parser
import tree_sitter_rust
assert sys.version_info[:2]==(3,13) and version('tree-sitter')=='0.25.2' and version('tree-sitter-rust')=='0.24.2'
cli=argparse.ArgumentParser();cli.add_argument('--output',type=Path,required=True);args=cli.parse_args();root=Path.cwd()
spec=importlib.util.spec_from_file_location('metrics',root/'scripts/refactor_metrics.py');metrics=importlib.util.module_from_spec(spec);spec.loader.exec_module(metrics)
parser=Parser(Language(tree_sitter_rust.language()));sha=lambda b:hashlib.sha256(b).hexdigest();lanes={}
paths=['crates/statechronicle/examples/custom_domains.rs','crates/statechronicle/examples/common/domains.rs']
for lane,ref in [('original','c418e98'),('candidate','e880a2a')]:
 revision=subprocess.check_output(['git','rev-parse',ref],text=True).strip();files={}
 for path in paths:
  data=subprocess.check_output(['git','show',revision+':'+path]);analysis=metrics.analyze(data,path,parser)
  files[path]=dict(source_sha256=sha(data),physical_lines=len(data.splitlines()),syntax_lines=analysis['syntax_lines'],lexical_tokens=analysis['lexical_tokens'])
 domains=subprocess.check_output(['git','show',revision+':'+paths[1]]);literals=re.findall(rb'"((?:configuration|equipment)\.[a-z_]+)"',domains)
 lanes[lane]=dict(revision=revision,files=files,operation_literal_occurrences=len(literals),distinct_operations=len(set(literals)))
assert lanes['original']['operation_literal_occurrences']==14 and lanes['candidate']['operation_literal_occurrences']==7
result=dict(runner_sha256=sha(Path(__file__).read_bytes()),shared_metric_runner_sha256=sha((root/'scripts/refactor_metrics.py').read_bytes()),lanes=lanes,command_construction_steps=dict(original=['canonicalize native command','assemble profile/schema/command envelope','derive operation separately'],candidate=['encode_command returns checked operation and canonical envelope']),limitations=['Same pure planning example, not a claim that signed durable setup requires fewer total trust decisions than original planning.','Counts include native rule/codec implementation; macro expansion is not counted as handwritten boilerplate.','Physical lines include comments/blanks; syntax lines exclude them using the pinned existing metric parser.','Operation mapping and envelope equivalence are compiled/behavior-tested by frozen external command overlays; this measurement does not replace those tests.','Human onboarding time is not measured.'])
args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
