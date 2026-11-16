#!/usr/bin/env python3
"""Finite exact-worktree qualification of compatibility batch completion."""
import gzip, hashlib, json, os, re, subprocess, time
from pathlib import Path
root=Path(subprocess.check_output(['git','rev-parse','--show-toplevel'],text=True).strip())
out=root/'docs/0.3/evidence/f2/batch-completion/qualification-e8d4c8f';out.mkdir(parents=True,exist_ok=False)
def sha(data):return hashlib.sha256(data).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
revision=git('rev-parse','HEAD').decode().strip()
paths=git('ls-files','crates','fuzz','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','ci/compat-0.2','ci/wire-0.2').decode().splitlines()
hashes={name:sha((root/name).read_bytes()) for name in paths}
assert not git('diff','HEAD','--',*paths)
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/profile-guards/worktree'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(key,None)
m={'candidate_commit':revision,'source_sha256':hashes,'rustc':subprocess.check_output(['rustc','-vV'],cwd=root,text=True),'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'checks':[],'status':'running','limitations':['Finite root tests; fuzz binaries compile under Clippy but are not executed as unbounded tests.','External original parity, portable consumers and benchmark are recorded separately.']}
(out/'runner.py').write_bytes(Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,extra=None):
 start=time.monotonic();result=subprocess.run(command,cwd=root,env=env| (extra or {}),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=900)
 data=gzip.compress(result.stdout,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(data)
 record={'label':label,'command':command,'extra_environment':extra or {},'exit_code':result.returncode,'seconds':time.monotonic()-start,'artifact':artifact,'sha256':sha(data)}
 if label in ['finite-tests','doctests']:
  record['passed_tests']=sum(int(n) for n in re.findall(rb'test result: ok\. (\d+) passed;',result.stdout))
 m['checks'].append(record);save();print(label,result.returncode,record.get('passed_tests',''),flush=True)
 assert result.returncode==0,label
 if label in ['finite-tests','doctests']:assert record['passed_tests']==(884 if label=='finite-tests' else 16)
save()
try:
 check('fmt',['cargo','fmt','--all','--','--check'])
 check('finite-tests',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--all-targets','--all-features','--locked','--offline'])
 check('doctests',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--doc','--all-features','--locked','--offline'])
 check('clippy',['cargo','clippy','--keep-going','--workspace','--all-targets','--all-features','--locked','--offline','--','-D','warnings'])
 check('rustdoc',['cargo','doc','--workspace','--no-deps','--all-features','--locked','--offline'],{'RUSTDOCFLAGS':'-D warnings'})
 check('diff-check',['git','diff','--check'])
 assert all(sha((root/name).read_bytes())==digest for name,digest in hashes.items())
 assert not git('diff','HEAD','--',*paths)
 assert git('rev-parse','HEAD').decode().strip()==revision
 m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(out.iterdir()) if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
