"""Recompute original/current syntax metrics with the identical pinned parser."""
import gzip,hashlib,json,subprocess,tempfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest();checks=[]
with tempfile.TemporaryDirectory(prefix='statechronicle-metrics-verify-') as temporary:
    for name in ['baseline-source-metrics.json','source-metrics.json']:
        expected=json.loads((out/name).read_text());path=Path(temporary)/name
        command=['uv','run','--python','3.13','--with','tree-sitter==0.25.2','--with','tree-sitter-rust==0.24.2','python','scripts/refactor_metrics.py','--revision',expected['revision'],'--output',str(path)]
        r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);artifact=name.removesuffix('.json')+'-recompute.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/artifact).write_bytes(data)
        assert r.returncode==0,r.stdout.decode(errors='replace');actual=json.loads(path.read_text());assert actual==expected,name
        assert actual['runner_sha256']==sha((root/'scripts/refactor_metrics.py').read_bytes())
        for key,total in actual['totals'].items():assert sum(f[key] for f in actual['files'].values())==total
        checks.append(dict(command=command,exit_code=r.returncode,artifact=artifact,sha256=sha(data),metric_report_sha256=sha((out/name).read_bytes()),exact_recompute_match=True))
for report,driver in [('complexity-metrics.json','measure_complexity.py'),('consumer-setup-metrics.json','measure_consumer_setup.py')]:
    with tempfile.TemporaryDirectory(prefix='statechronicle-supplement-verify-') as temporary:
        expected=json.loads((out/report).read_text());path=Path(temporary)/report
        command=['uv','run','--python','3.13','--with','tree-sitter==0.25.2','--with','tree-sitter-rust==0.24.2','python',str(out/driver),'--output',str(path)]
        r=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);artifact=report.removesuffix('.json')+'-recompute.log.gz';data=gzip.compress(r.stdout,mtime=0);(out/artifact).write_bytes(data)
        assert r.returncode==0,r.stdout.decode(errors='replace');assert json.loads(path.read_text())==expected
        assert expected['runner_sha256']==sha((out/driver).read_bytes())
        checks.append(dict(command=command,exit_code=r.returncode,artifact=artifact,sha256=sha(data),metric_report_sha256=sha((out/report).read_bytes()),exact_recompute_match=True))
result=dict(status='passed',checks=checks,limitations=['Parser syntax counts, including macro shells, are not compiled API or machine-code complexity. The separate cyclomatic supplement is an explicitly bounded source approximation.','Growth accounting and retained-duplicate decisions are reviewed separately.'])
(out/'metrics-verification.json').write_text(json.dumps(result,indent=2)+'\n');print('Original/current exact pinned-parser recomputations passed.')
