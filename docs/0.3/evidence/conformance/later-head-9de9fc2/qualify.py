from pathlib import Path
import subprocess,tarfile,io,os,json,time,concurrent.futures,hashlib
root=Path.cwd();base=root/'target/later-head/qualification';base.mkdir(exist_ok=True)
revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
archive=subprocess.check_output(['git','archive',revision]);source=base/'source';source.mkdir(exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(archive)) as tf:tf.extractall(source,filter='data')
manifest={'candidate_commit':revision,'archive_sha256':hashlib.sha256(archive).hexdigest(),'compilers':{t:subprocess.check_output(['rustc','+'+t,'-Vv'],text=True) for t in ['stable','1.85.0']},'checks':[]}
def lane(tool):
 checks=[]
 for label,args in [('tests',['test','--all-targets','--locked','--offline','--','--nocapture']),('doctests',['test','--doc','--locked','--offline']),('clippy',['clippy','--all-targets','--locked','--offline','--','-D','warnings']),('rustdoc',['doc','--no-deps','--locked','--offline']),('format',['fmt','--','--check'])]:
  # Cargo fmt has no --manifest-path after the subcommand flags.
  command=['cargo','+'+tool,args[0],'--manifest-path',str(source/'ci/conformance/Cargo.toml'),*args[1:]]
  overrides={'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8','CARGO_TARGET_DIR':str(base/tool)}
  if label=='rustdoc':overrides['RUSTDOCFLAGS']='-D warnings'
  start=time.monotonic();log=base/f'{tool}-{label}.log'
  with log.open('w') as f:rc=subprocess.run(command,cwd=source,env=dict(os.environ,**overrides),stdout=f,stderr=subprocess.STDOUT).returncode
  checks.append({'label':tool+'-'+label,'command':command,'environment':overrides,'exit_code':rc,'seconds':time.monotonic()-start,'log':log.name,'sha256':hashlib.sha256(log.read_bytes()).hexdigest()})
  print(tool,label,rc,flush=True)
  if rc:break
 return checks
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
 a=pool.submit(lane,'stable');b=pool.submit(lane,'1.85.0');manifest['checks']=a.result()+b.result()
(base/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
assert len(manifest['checks'])==10 and all(c['exit_code']==0 for c in manifest['checks'])
