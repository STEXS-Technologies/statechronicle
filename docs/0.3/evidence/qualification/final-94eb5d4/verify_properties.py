"""Verify archived source and all three raw seeded property executions."""
import gzip,hashlib,io,json,re,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;directory=out/'properties';m=json.loads((directory/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
assert m['status']=='passed' and m['source_unchanged']
archive=subprocess.check_output(['git','archive',m['candidate_commit'],'--',*m['archive_paths']]);assert sha(archive)==m['archive_sha256']
with tarfile.open(fileobj=io.BytesIO(archive)) as handle:
    hashes={f.name:sha(handle.extractfile(f).read()) for f in handle if f.isfile()}
assert hashes==m['source_sha256']
assert [c['seed'] for c in m['checks']]==[7331,42,65537]
total=0
for check in m['checks']:
    assert check['exit_code']==0
    data=(directory/check['artifact']).read_bytes();assert sha(data)==check['sha256'];raw=gzip.decompress(data)
    counts=[int(n) for n in re.findall(rb'test result: ok\. (\d+) passed',raw)]
    assert counts==check['test_counts']==[6,5,7,3,3,7,5,12,9]
    assert not re.search(rb'test result: FAILED|Failed to parse|panicked at',raw)
    total+=sum(counts)
    if check['seed']==7331:
        for crate in ['profile','core','domain','accumulator','intent','ports','profiles','commit','proof','executor']:
            assert re.search(rb'Compiling statechronicle-'+crate.encode()+rb' v',raw),crate
assert total==171
result=dict(status='passed',manifest_sha256=sha((directory/'manifest.json').read_bytes()),seeds=[7331,42,65537],test_executions=total,target_binaries_per_seed=9,limitations=m['limitations'])
(directory/'verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
