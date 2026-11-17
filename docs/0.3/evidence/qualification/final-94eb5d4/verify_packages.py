"""Verify final crate bytes and successful isolated all-target package checks."""
import gzip, hashlib, io, json
from pathlib import Path
import subprocess, tarfile
out=Path('docs/0.3/evidence/packages/20261116T180422Z-1975357');m=json.loads((out/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
def require(ok,message):
 if not ok:raise ValueError(message)
require(m['status']=='passed' and m['build_isolation']=='fresh per-archive target directory','package status/isolation')
require({c['label'] for c in m['checks']}=={'metadata','package','resolve','unpacked-metadata','tests','doctests','clippy'},'all required checks')
for c in m['checks']:require(c['exit_code']==0 and sha((out/c['log']).read_bytes())==c['log_sha256'],'check/log '+c['label'])
require(len(m['archives'])==13,'all library archives')
for name,digest in m['archives'].items():
 data=(out/name).read_bytes();require(sha(data)==digest,'archive '+name)
 package=name.removesuffix('-0.2.0.crate');prefix=package+'-0.2.0/'
 with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as tar:
  actual={p.name.removeprefix(prefix):tar.extractfile(p).read() for p in tar if p.isfile() and p.name.startswith(prefix+'src/')}
 archive=subprocess.check_output(['git','archive',m['candidate_commit'],'--','crates/'+package+'/src'])
 with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
  expected={p.name.removeprefix('crates/'+package+'/'):tar.extractfile(p).read() for p in tar if p.isfile()}
 require(actual==expected,'complete exact original source in archive '+package)
print('Verified thirteen complete source archives and all fresh isolated package checks/log hashes.')
