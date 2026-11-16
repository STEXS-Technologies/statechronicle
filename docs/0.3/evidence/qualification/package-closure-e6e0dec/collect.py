from pathlib import Path
import json,gzip,hashlib,subprocess,re
root=Path.cwd();base=root/'target/package-gate';out=root/'docs/0.3/evidence/qualification/package-closure-e6e0dec';out.mkdir(parents=True,exist_ok=True)
sha=lambda b:hashlib.sha256(b).hexdigest();artifacts={}
for p in sorted(base.glob('*')):
 if p.is_file() and p.suffix in ['.log','.json','.py']:
  name=p.name;data=p.read_bytes()
  if p.suffix=='.log':name+='.gz';data=gzip.compress(data,mtime=0)
  (out/name).write_bytes(data);artifacts[name]=sha(data)
# Keep the actual failing umbrella archive from the initial pre-fix probe.
p=base/'build/package/statechronicle-0.2.0.crate';data=p.read_bytes();(out/'pre-fix-statechronicle-0.2.0.crate').write_bytes(data);artifacts['pre-fix-statechronicle-0.2.0.crate']=sha(data)
packaging=[]
for p in sorted((root/'docs/0.3/evidence/packages').glob('*/manifest.json')):
 m=json.loads(p.read_text());counts={}
 for label in ['tests','doctests']:
  log=p.parent/(label+'.log.gz')
  if log.exists():counts[label]=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',gzip.decompress(log.read_bytes()).decode())))
 packaging.append({'manifest':str(p.relative_to(root)),'candidate_commit':m['candidate_commit'],'status':m['status'],'counts':counts,'sha256':sha(p.read_bytes())})
assert len(packaging)==2 and [x['status'] for x in packaging]==['failed','passed']
assert packaging[-1]['counts']=={'tests':884,'doctests':16}
compat=root/'docs/0.3/evidence/compat/20261116T102804Z-3616467/manifest.json';assert json.loads(compat.read_text())['status']=='passed'
unchanged=['crates/*/src','Cargo.toml','Cargo.lock','ci/compat-0.2','ci/wire-0.2','docs/CONFORMANCE/custom_domains.json','clippy.toml']
assert not subprocess.check_output(['git','diff','4c042e4','e6e0dec','--',*unchanged])
manifest={'comparison_commit':'4c042e40527334716edb0c280cb81f66562d91c9','fixture_closure_candidate':'907c35901f0860f71d8fdcaf5f6318c9384c1721','qualification_candidate':'e6e0decb3a68b1bae54c827e437a5f281b11a2e6','package_manifests':packaging,'compatibility_manifest':str(compat.relative_to(root)),'compatibility_manifest_sha256':sha(compat.read_bytes()),'unchanged_scopes':unchanged,'artifacts_sha256':artifacts,'limitations':['Self-contained local archive qualification; no crates.io publication or registry-service test.','Versions remain 0.2.0; version changes require new archive qualification.','Initial unpacked archive tests already passed after fixture correction; initial strict lint used different default policy and failed.','Final checker copies the existing committed lint configuration into the synthetic workspace; no threshold or suppression changed.','Fixture duplication is test-only and provenance checked; not a production footprint reduction.','Broader F0-F8 gates remain incomplete.']}
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(out.relative_to(root),flush=True)
