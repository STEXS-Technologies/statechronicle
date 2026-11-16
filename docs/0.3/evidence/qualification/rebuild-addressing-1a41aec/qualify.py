from pathlib import Path
import subprocess,os,json,time,concurrent.futures,hashlib
root=Path.cwd();base=root/'target/rebuild-addressing/qualification';base.mkdir(exist_ok=True)
revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
subprocess.run(['git','diff','--exit-code',revision,'--','crates','Cargo.toml','Cargo.lock','fuzz'],check=True)
def lane(tool,target):
 out=[]
 commands=[('tests',['test','--workspace','--exclude','statechronicle-fuzz','--all-targets','--all-features','--locked','--offline']),('doctests',['test','--workspace','--exclude','statechronicle-fuzz','--doc','--all-features','--locked','--offline']),('clippy',['clippy','--keep-going','--workspace','--all-targets','--all-features','--locked','--offline','--','-D','warnings']),('rustdoc',['doc','--workspace','--all-features','--no-deps','--locked','--offline']),('format',['fmt','--all','--','--check']),('example',['run','-p','statechronicle','--example','custom_domains','--locked','--offline'])]
 for label,args in commands:
  command=['cargo','+'+tool,*args];overrides={'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8','CARGO_TARGET_DIR':str(root/target)}
  if label=='rustdoc':overrides['RUSTDOCFLAGS']='-D warnings'
  env=dict(os.environ,**overrides);log=base/f'{tool}-{label}.log';start=time.monotonic()
  with log.open('w') as f:rc=subprocess.run(command,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
  out.append({'label':tool+'-'+label,'command':command,'environment':overrides,'exit_code':rc,'seconds':time.monotonic()-start,'log':log.name,'sha256':hashlib.sha256(log.read_bytes()).hexdigest()})
  print(tool,label,rc,flush=True)
  if rc:break
 return out
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
 a=pool.submit(lane,'stable','target/profile-guards/worktree');b=pool.submit(lane,'1.85.0','target/profile-guards/msrv');checks=a.result()+b.result()
manifest={'candidate_commit':revision,'compilers':{t:subprocess.check_output(['rustc','+'+t,'-Vv'],text=True) for t in ['stable','1.85.0']},'checks':checks}
(base/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
assert len(checks)==12 and all(x['exit_code']==0 for x in checks)
