"""Repeat all named property integration targets on immutable candidate source."""
import gzip,hashlib,io,json,os
from pathlib import Path
import re,subprocess,tarfile,tempfile,time
root=Path.cwd();out=Path(__file__).resolve().parent/'properties';out.mkdir()
revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz']
archive=subprocess.check_output(['git','archive',revision,'--',*paths]);sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(candidate_commit=revision,archive_paths=paths,archive_sha256=sha(archive),status='running',checks=[],limitations=['PROPTEST_CASES supplies default cases only; target-local explicit case counts retain precedence.','Recorded seeds vary generated cases, not OS thread scheduling or all possible inputs.'])
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-properties-') as temp:
  source=Path(temp)/'source';source.mkdir()
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
  def inputs():return {str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
  m['source_sha256']=inputs()
  env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(Path(temp)/'target'),PROPTEST_CASES='1024')
  m['environment']={k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR','PROPTEST_CASES']}
  command=['cargo','test','--workspace','--exclude','statechronicle-fuzz','--test','property','--test','amount_property','--all-features','--locked','--offline','--','--nocapture']
  for seed in [7331,42,65537]:
   env['PROPTEST_RNG_SEED']=str(seed);start=time.monotonic();p=subprocess.run(command,cwd=source,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
   data=gzip.compress(p.stdout,mtime=0);name=str(seed)+'.log.gz';(out/name).write_bytes(data)
   counts=[int(n) for n in re.findall(rb'test result: ok\. (\d+) passed',p.stdout)]
   m['checks'].append(dict(seed=seed,command=command,exit_code=p.returncode,seconds=time.monotonic()-start,artifact=name,sha256=sha(data),test_counts=counts));save()
   if p.returncode or len(counts)!=9 or b'Failed to parse' in p.stdout:raise RuntimeError('property seed '+str(seed)+' failed/target count mismatch')
   print(seed,sum(counts),'property tests passed',flush=True)
  m['source_unchanged']=inputs()==m['source_sha256'];assert m['source_unchanged'];m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
