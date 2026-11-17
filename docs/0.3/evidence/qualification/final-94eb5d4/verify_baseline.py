"""Verify original-release provenance and retained baseline successes/failures."""
import gzip,hashlib,io,json,re,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;directory=root/'docs/0.3/evidence/baseline';sha=lambda b:hashlib.sha256(b).hexdigest()
revision='c418e98b91720188158c531cd5c2c4b8ab71cbcf'
assert subprocess.check_output(['git','rev-parse','v0.2.0^{commit}'],text=True).strip()==revision
archive=subprocess.check_output(['git','archive',revision])
with tarfile.open(fileobj=io.BytesIO(archive)) as handle:source={f.name:sha(handle.extractfile(f).read()) for f in handle if f.isfile()}
results=[]
for stamp,expected in [('20261116T014840Z-2747068','passed'),('20261116T015301Z-2774962','failed'),('20261116T020259Z-2851508','passed')]:
    path=directory/stamp;m=json.loads((path/'manifest.json').read_text());assert m['baseline_commit']==revision and m['status']==expected
    assert m['archive_sha256']==sha(archive);assert m['source_sha256']==source
    logs=[]
    for check in m['checks']:
        data=(path/check['artifact']).read_bytes();assert sha(data)==check['sha256'];logs.append(gzip.decompress(data))
        if expected=='passed':assert check['exit_code']==0
        else:assert check['exit_code']!=0
    raw=b'\n'.join(logs)
    if stamp=='20261116T014840Z-2747068':assert sum(map(int,re.findall(rb'test result: ok\. (\d+) passed',raw)))==794
    if expected=='failed':
        assert b'RUSTSEC-2026-0486' in raw and b'RUSTSEC-2026-0386' in raw
    results.append(dict(path=str(path.relative_to(root)),status=expected,manifest_sha256=sha((path/'manifest.json').read_bytes())))
result=dict(status='passed',baseline_commit=revision,checks=results,limitations=['Original release dependency-policy failure is preserved, not a successful old release gate.','Candidate security policy is qualified separately with the approved dependency refresh.'])
(out/'baseline-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
