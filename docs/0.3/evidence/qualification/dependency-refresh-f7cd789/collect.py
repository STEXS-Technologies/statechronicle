from pathlib import Path
import hashlib,json,gzip,shutil,subprocess,tomllib,tarfile,io,re
root=Path.cwd();base=root/'target/release-audit';out=root/'docs/0.3/evidence/qualification/dependency-refresh-f7cd789';out.mkdir(parents=True,exist_ok=True)
sha=lambda b:hashlib.sha256(b).hexdigest()
artifacts={}
def put(name,data):
 p=out/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);artifacts[name]=sha(data)
for scope in ['qualification','support','fuzz-smoke']:
 for p in sorted((base/scope).glob('*')):
  if p.is_file() and p.suffix in ['.log','.json']:
   name=scope+'/'+p.name
   if p.suffix=='.log':name+='.gz';data=gzip.compress(p.read_bytes(),mtime=0)
   else:data=p.read_bytes()
   put(name,data)
for p in sorted(base.glob('*')):
 if p.is_file() and p.suffix in ['.log','.json','.stderr']:
  put('preflights/'+p.name+'.gz',gzip.compress(p.read_bytes(),mtime=0))
for name in ['qualify.py','support.py','fuzz-smoke.py','collect.py']:put(name,(base/name).read_bytes())
# Preserve both unchanged original local seed inputs and the resulting corpus.
for label,source in [('initial-corpus',root/'fuzz/corpus'),('final-corpus',base/'fuzz-smoke')]:
 buf=io.BytesIO();files={}
 with tarfile.open(fileobj=buf,mode='w') as tar:
  for p in sorted(source.rglob('*')):
   if not p.is_file() or label=='final-corpus' and p.parent==source:continue
   data=p.read_bytes();name=str(p.relative_to(source));files[name]=sha(data);info=tarfile.TarInfo(name);info.size=len(data);info.mtime=0;info.mode=0o644;tar.addfile(info,io.BytesIO(data))
 put(label+'.tar.gz',gzip.compress(buf.getvalue(),mtime=0));put(label+'-sha256.json',(json.dumps(files,indent=2)+'\n').encode())
candidate='f7cd78941febb4e3157fa3047c7f74c3ae68e380';tutorial_fix='0dfd6ea8f54c92b954479cd9049462dcd400f1d0';before='d51503209950a840d1fe7875f00d43d9101c1405'
files=subprocess.check_output(['git','ls-files','--','crates','fuzz','Cargo.toml','Cargo.lock','rust-toolchain.toml','ci/conformance','tutorials','.github','scripts/check_embedded.sh','scripts/check_tutorials.py'],text=True).splitlines()
source={name:sha((root/name).read_bytes()) for name in files}
# Prove library Rust source did not change with the dependency refresh.
unchanged={}
for name in files:
 if name.startswith('crates/') and '/src/' in name and name.endswith('.rs'):
  old=subprocess.check_output(['git','show',before+':'+name]);assert old==(root/name).read_bytes();unchanged[name]=sha(old)
assert not subprocess.check_output(['git','diff',candidate,tutorial_fix,'--','crates','fuzz','Cargo.toml','Cargo.lock','ci/conformance','rust-toolchain.toml','.github'])
old=tomllib.loads(subprocess.check_output(['git','show',before+':Cargo.lock'],text=True));new=tomllib.loads(Path('Cargo.lock').read_text())
pkg=lambda x:{(p['name'],p['version'],p.get('source'),p.get('checksum')) for p in x['package'] if p.get('source')}
runs=[]
for p in sorted((root/'docs/0.3/evidence/tutorials').glob('*-20261116T10*/manifest.json')):
 m=json.loads(p.read_text())
 if m['candidate_commit'] in [candidate,tutorial_fix]:runs.append({'path':str(p.relative_to(root)),'candidate_commit':m['candidate_commit'],'status':m['status'],'sha256':sha(p.read_bytes())})
compat=root/'docs/0.3/evidence/compat/20261116T100831Z-2805936/manifest.json';cm=json.loads(compat.read_text());assert cm['status']=='passed'
qual=json.loads((base/'qualification/manifest.json').read_text());assert len(qual['checks'])==6 and all(x['exit_code']==0 for x in qual['checks'])
support=json.loads((base/'support/manifest.json').read_text());assert len(support['checks'])==15 and all(x['passed'] for x in support['checks'])
fuzz=json.loads((base/'fuzz-smoke/manifest.json').read_text());assert len(fuzz['runs'])==25 and all(x['exit_code']==0 for x in fuzz['runs'])
manifest={'candidate_commit':candidate,'tutorial_cleanup_commit':tutorial_fix,'comparison_commit':before,'toolchain_source':'https://blog.rust-lang.org/releases/latest/','advisory_sources':['https://rustsec.org/advisories/RUSTSEC-2026-0486.html','https://rustsec.org/advisories/RUSTSEC-2026-0386.html'],'source_sha256':source,'unchanged_library_rust_src_sha256':unchanged,'removed_registry_packages':sorted(pkg(old)-pkg(new)),'added_registry_packages':sorted(pkg(new)-pkg(old)),'workspace_counts':{label:sum(map(int,re.findall(r'test result: ok\. (\d+) passed',(base/f'qualification/1.100.0-{label}.log').read_text()))) for label in ['tests','doctests']},'compatibility_manifest':str(compat.relative_to(root)),'compatibility_manifest_sha256':sha(compat.read_bytes()),'tutorial_manifests':runs,'artifacts_sha256':artifacts,'limitations':['Dependency and scoped qualification checkpoint, not complete F0-F8 release qualification.','884 finite and 16 doc tests run at f7cd789; later cleanup changes only tutorial Rust, whose full isolated checks were rerun.','Only current active locks audited/updated; frozen original and historical evidence locks unchanged.','Fuzzing is bounded ASAN smoke; exact per-target counts and time bounds are in logs.','Library Rust unchanged; dependency/compiler performance and binary-size impact not measured here.','Initial fetch attempts failed due to ambient sccache; corrected RUSTC_WRAPPER-empty commands and logs retained.','Durable tutorial initial strict lint failure retained and subsequently corrected without suppression.','No PR, workflow dispatch, tag or publication authorized.']}
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');print(out.relative_to(root),flush=True)
