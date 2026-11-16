from pathlib import Path
import subprocess,tempfile,shutil,re,json,gzip,hashlib,os,concurrent.futures
root=Path.cwd(); dest=root/'docs/0.3/evidence/documentation/migration-b5d00d7'; dest.mkdir(parents=True,exist_ok=True)
sha=lambda b:hashlib.sha256(b).hexdigest()
rev=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(); assert rev=='b5d00d747e9ba834648399f21bff0547ba1f467a'
files=subprocess.check_output(['git','diff','--name-only','HEAD^','HEAD'],text=True).splitlines(); links=[]
for f in files:
 for link in re.findall(r'\]\(([^)]+)\)',Path(f).read_text()):
  if '://' in link or link.startswith('#'): continue
  assert (Path(f).parent/link.split('#')[0]).exists(), (f,link)
  links.append([f,link])
snippet=re.search(r'```rust\n(.*?)```',Path('docs/0.3/migration.md').read_text(),re.S).group(1)
# Imports are documentation about path resolution, so suppress only unused imports.
source='#![allow(unused_imports)]\n'+snippet+'\nfn main() {}\n'
def check(tc):
 with tempfile.TemporaryDirectory(prefix='statechronicle-migration-') as temp:
  app=Path(temp); original=root/'tutorials/minimal-native-domain'
  shutil.copy(original/'Cargo.lock',app/'Cargo.lock'); (app/'Cargo.toml').write_text((original/'Cargo.toml').read_text().replace('../../crates/statechronicle',str(root/'crates/statechronicle')))
  (app/'src').mkdir(); (app/'src/main.rs').write_text(source)
  cmd=['cargo','+'+tc,'check','--all-targets','--locked','--offline']; env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/migration-docs'/tc))
  result=subprocess.run(cmd,cwd=app,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  log=dest/(tc+'.log.gz'); log.write_bytes(gzip.compress(result.stdout,mtime=0))
  record={'toolchain':subprocess.check_output(['rustc','+'+tc,'-Vv'],text=True),'command':cmd,'exit_code':result.returncode,'log_sha256':sha(log.read_bytes()),'log':log.name,'lock_unchanged':(app/'Cargo.lock').read_bytes()==(original/'Cargo.lock').read_bytes()}
  print(tc,result.returncode,flush=True); return record
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool: records=list(pool.map(check,['stable','1.85.0']))
assert all(x['exit_code']==0 and x['lock_unchanged'] for x in records)
unchanged=['crates/*/src','Cargo.toml','Cargo.lock','fuzz','tutorials','.github']
assert not subprocess.check_output(['git','diff','HEAD^','HEAD','--',*unchanged])
manifest={'candidate_commit':rev,'previous_commit':subprocess.check_output(['git','rev-parse','HEAD^'],text=True).strip(),'source_sha256':{f:sha(Path(f).read_bytes()) for f in files},'local_links_checked':links,'migration_import_source':source,'import_checks':records,'unchanged_scopes':unchanged,'limitations':['Documentation-only checkpoint; import check proves public path resolution, not full runtime behavior.','Both standalone tutorials retain previous exact-source qualification; no library test rerun claimed.','Links check local target existence, not rendered layout or remote network availability.','Broader F0-F8 qualification remains open.']}
(dest/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Evidence:',dest.relative_to(root),flush=True)
