"""Recheck complete strict actual-original/current semantic traces and provenance."""
import gzip,hashlib,io,json
from pathlib import Path
import subprocess,tarfile
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest()
original=subprocess.check_output(['git','rev-parse','v0.2.0^{commit}'],text=True).strip()
def require(ok,message):
 if not ok:raise ValueError(message)
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz']
locations={
 'proof_service':'docs/0.3/evidence/maintenance/proof-service/20261116T182114Z-2235511',
 'executor':'docs/0.3/evidence/f2/executor-order/20261116T182259Z-2257784',
 'settlement':'docs/0.3/evidence/f2/executor-order/20261116T182259Z-2257788',
 'profiles':'docs/0.3/evidence/maintenance/builtin-profiles/20261116T182259Z-2257796',
 'outbox':'docs/0.3/evidence/maintenance/outbox-preflight/20261116T182259Z-2257811',
}
# Select only a terminal passing string campaign against the actual original,
# never an interim dependency-control lane or an interrupted metadata attempt.
for p in sorted((root/'docs/0.3/evidence/maintenance/string-traits').glob('*/manifest.json')):
 m=json.loads(p.read_text())
 if m.get('status')=='passed' and len(m.get('revisions',[]))==2 and m['revisions'][0]==subprocess.check_output(['git','rev-parse','v0.2.0'],text=True).strip() and m['revisions'][1]==subprocess.check_output(['git','rev-parse','e880a2a'],text=True).strip():locations['string_traits']=str(p.parent.relative_to(root))
require('string_traits' in locations,'strict current string-trait campaign missing')
expected_rows={'proof_service':2352,'executor':23544,'settlement':10128,'profiles':296604,'outbox':5688,'string_traits':10305}
verified=[]
for fixture,location in locations.items():
 directory=root/location;m=json.loads((directory/'manifest.json').read_text())
 require(m['status']=='passed' and m['different_rows']==0 and len(m['lanes'])==2,'status/lanes '+fixture)
 require(subprocess.check_output(['git','rev-parse',m['lanes'][0]['revision']+'^{commit}'],text=True).strip()==original,'actual original source')
 require(not any(v for k,v in m.items() if k.startswith('negative_control')),'positive source only')
 for name,digest in m['artifacts_sha256'].items():require(sha((directory/name).read_bytes())==digest,'artifact '+name)
 for c in m['checks']:require(c['exit_code']==0 and sha((directory/c['artifact']).read_bytes())==c['sha256'],'check/log '+c['label'])
 fixture_name='fixture.rs' if fixture=='string_traits' else fixture+'.rs'
 require(sha((directory/fixture_name).read_bytes())==m['fixture_sha256'],'fixture provenance')
 traces=[]
 for index,lane in enumerate(m['lanes']):
  archive=subprocess.check_output(['git','archive',lane['revision'],'--',*paths]);require(sha(archive)==lane['archive_sha256'],'archive '+fixture)
  with tarfile.open(fileobj=io.BytesIO(archive)) as t:inputs={p.name:sha(t.extractfile(p).read()) for p in t if p.isfile()}
  require(inputs==lane['source_sha256'],'immutable source '+fixture)
  build=next(c for c in m['checks'] if c['label']==str(index)+'-build')
  compiled=gzip.decompress((directory/build['artifact']).read_bytes()).decode()
  libraries=['statechronicle','statechronicle-core','statechronicle-domain','statechronicle-intent','statechronicle-executor','statechronicle-accumulator','statechronicle-ports','statechronicle-profiles','statechronicle-profile','statechronicle-commit','statechronicle-proof','statechronicle-portable','statechronicle-index']
  require(all('Compiling '+package+' v0.2.0 (' in compiled for package in libraries),'observed complete library recompilation '+fixture)
  check=next(c for c in m['checks'] if c['label']==str(index)+'-trace')
  raw=gzip.decompress((directory/check['artifact']).read_bytes());require(sha(raw)==lane['trace_sha256'],'trace hash')
  trace=json.loads(raw);require(len(trace)==lane['rows']==expected_rows[fixture],'exact case count')
  require(len({json.dumps(row['case'],sort_keys=True) for row in trace})==len(trace),'unique case identities')
  traces.append(trace)
 require(traces[0]==traces[1],'exact errors/effects/callback trace equality '+fixture)
 verified.append(dict(fixture=fixture,rows=expected_rows[fixture],location=location,manifest_sha256=sha((directory/'manifest.json').read_bytes()),candidate=lane['revision']))
 print(fixture,expected_rows[fixture],'strict original/current rows verified',flush=True)
(out/'semantic-verification.json').write_text(json.dumps(dict(status='passed',original_commit=original,verified=verified,total_distinct_cases=sum(i['rows'] for i in verified),limitations=['Bounded characterization fixtures, not exhaustive input semantics.','Original signed wire/history, protected API fuzz, properties, crash and concurrency have separate evidence.']),indent=2)+'\n')
