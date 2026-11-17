"""Verify current frozen-consumer and raw coverage records independently."""
import gzip,hashlib,io,json,re,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git',*args])
def checks(base,m):
    assert m['status']=='passed';logs={}
    for check in m['checks']:
        assert check['exit_code']==0
        data=(base/check['artifact']).read_bytes();assert sha(data)==check['sha256'];logs[check['label']]=gzip.decompress(data)
    return logs
base=root/'docs/0.3/evidence/compat/20261116T180227Z-1914823';m=json.loads((base/'manifest.json').read_text());logs=checks(base,m)
assert m['baseline_commit']==git('rev-parse','v0.2.0^{commit}').decode().strip()
assert m['runner_sha256']==sha((root/'scripts/refactor_compat.py').read_bytes())
assert [c['label'] for c in m['checks']]==[lane+'-'+check for lane in ['baseline','candidate'] for check in ['resolve','tests','portable-resolve','portable-tests','bare-metal']]
for path,h in m['fixture_sha256'].items():assert sha((root/'ci/compat-0.2'/path).read_bytes())==h
provenance=json.loads((root/'ci/compat-0.2/provenance.json').read_text())
for path,item in provenance['files'].items():assert sha(git('show',m['baseline_commit']+':'+item['original_path']))==item['sha256']==m['fixture_sha256'][path]
origin=json.loads((root/'ci/wire-0.2/provenance.json').read_text());assert origin['producer_commit']==m['baseline_commit'];assert sha((root/'ci/wire-0.2/history.json').read_bytes())==origin['history_sha256']==m['wire_sha256']
assert sha((root/origin['producer_source_artifact']).read_bytes())==origin['producer_sha256']
for path,h in m['canonical_consumer_overlays'].items():assert sha((root/path).read_bytes())==h
counts={}
for lane in ['baseline','candidate']:
    for suffix,key in [('',lane+'_lock_sha256'),('-portable',lane+'_portable_lock_sha256')]:assert sha((base/(lane+suffix+'.lock')).read_bytes())==m[key]
    for label in ['tests','portable-tests']:
        raw=logs[lane+'-'+label];numbers=list(map(int,re.findall(rb'test result: ok\. (\d+) passed',raw)));assert numbers and sum(numbers)>0;assert b'test result: FAILED' not in raw
        counts[lane+'-'+label]=sum(numbers)
    assert b'Finished' in logs[lane+'-bare-metal']
compat=dict(status='passed',manifest_sha256=sha((base/'manifest.json').read_bytes()),test_executions=counts)
base=root/'docs/0.3/evidence/coverage/hosted-94eb5d4';m=json.loads((base/'manifest.json').read_text());logs=checks(base,m)
archive=git('archive',m['candidate'],'--',*m['archive_paths']);assert sha(archive)==m['archive_sha256']
with tarfile.open(fileobj=io.BytesIO(archive)) as handle:files={f.name:handle.extractfile(f).read() for f in handle if f.isfile()}
assert {p:sha(b) for p,b in files.items()}==m['source_sha256']
assert files['ci/coverage-baseline.json']==(base/'coverage-baseline.json').read_bytes()==(root/'ci/coverage-baseline.json').read_bytes()
floor=json.loads(files['ci/coverage-baseline.json'])['line_coverage_percent'];assert floor==92.4
reports={}
for name,item in m['reports'].items():
    raw=(base/name).read_bytes();assert sha(raw)==item['sha256'];data=json.loads(raw);assert len(data['data'])==1
    totals=data['data'][0]['totals']['lines'];assert all(totals[k]==item[k] for k in ['count','covered','percent'])
    rows=data['data'][0]['files'];assert sum(f['summary']['lines']['count'] for f in rows)==totals['count'];assert sum(f['summary']['lines']['covered'] for f in rows)==totals['covered']
    assert abs(totals['covered']/totals['count']*100-totals['percent'])<1e-10
    reports[name]=totals
assert reports['coverage.json']['percent']>=floor
result=dict(status='passed',compatibility=compat,coverage=dict(manifest_sha256=sha((base/'manifest.json').read_bytes()),unchanged_floor=floor,reports=reports),limitations=['Frozen consumer fixtures cover their declared source/protocol contracts, not all downstream code.','LLVM line attribution is not branch coverage or per-macro-instantiation coverage.'])
(out/'compat-coverage-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
