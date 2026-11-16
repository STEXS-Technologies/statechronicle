#!/usr/bin/env python3
"""Run local finite workspace gates on the exact committed proof candidate."""
import gzip,hashlib,json,os,subprocess,time
from pathlib import Path
root=Path(__file__).resolve().parents[1]
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
def sha(data):return hashlib.sha256(data).hexdigest()
revision=git('rev-parse','HEAD').decode().strip()
out=root/'docs/0.3/evidence/maintenance/proof-service'/('qualification-'+revision[:7]);out.mkdir(parents=True,exist_ok=False)
guards=['crates','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','fuzz','ci/compat-0.2','ci/wire-0.2']
assert not git('diff',revision,'--',*guards)
files=git('ls-files','--',*guards).decode().splitlines();hashes={p:sha((root/p).read_bytes()) for p in files}
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/profile-guards/worktree'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS']:env.pop(key,None)
m={'candidate_commit':revision,'source_sha256':hashes,'rustc':subprocess.check_output(['rustc','-vV'],text=True),'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'status':'running','checks':[],'limitations':['Finite hosted workspace gates; excludes unbounded fuzz binary execution.','No new exact-candidate extended fuzz, coverage, package publication or full performance-matrix claim.','Retained source guards include frozen compatibility fixtures and active root lock.']}
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,cmd,extra=None):
 start=time.monotonic();result=subprocess.run(cmd,cwd=root,env=dict(env,**(extra or {})),stdout=subprocess.PIPE,stderr=subprocess.STDOUT);data=gzip.compress(result.stdout,mtime=0);(out/(label+'.log.gz')).write_bytes(data)
 m['checks'].append({'label':label,'command':cmd,'extra_environment':extra or {},'exit_code':result.returncode,'seconds':time.monotonic()-start,'artifact':label+'.log.gz','sha256':sha(data)});save();print(label,result.returncode,flush=True)
 if result.returncode:raise RuntimeError(label+' failed')
save()
try:
 check('fmt',['cargo','fmt','--all','--','--check'])
 check('finite-tests',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--all-targets','--all-features','--locked','--offline'])
 check('doctests',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--doc','--all-features','--locked','--offline'])
 check('strict-clippy',['cargo','clippy','--keep-going','--workspace','--all-targets','--all-features','--locked','--offline','--','-D','warnings'])
 check('strict-rustdoc',['cargo','doc','--workspace','--no-deps','--all-features','--locked','--offline'],{'RUSTDOCFLAGS':'-D warnings'})
 check('guarded-source-diff',['git','diff','--exit-code',revision,'--',*guards])
 assert all(sha((root/p).read_bytes())==value for p,value in hashes.items())
 m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(out.iterdir()) if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
