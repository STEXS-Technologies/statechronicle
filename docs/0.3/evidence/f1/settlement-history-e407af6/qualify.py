import subprocess,pathlib,os,hashlib,json,gzip,io,tarfile,re
root=pathlib.Path.cwd();sha=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();parent=subprocess.check_output(['git','rev-parse','HEAD^'],text=True).strip()
out=root/'docs/0.3/evidence/f1'/('settlement-history-'+sha[:7]);out.mkdir(parents=True,exist_ok=True)
base=root/'target/settlement-history'/sha;base.mkdir(exist_ok=True)
def archive(rev,name):
 data=subprocess.check_output(['git','archive',rev]);source=base/name;source.mkdir(exist_ok=True)
 with tarfile.open(fileobj=io.BytesIO(data)) as t:t.extractall(source,filter='data')
 return source,hashlib.sha256(data).hexdigest()
source,archive_hash=archive(sha,'candidate');old,old_hash=archive(parent,'parent')
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target'))
manifest={'candidate_commit':sha,'parent_commit':parent,'source_archive_sha256':archive_hash,'parent_archive_sha256':old_hash,'source_sha256':{},'commands':[],'execution_source':'isolated git archives with no edits during qualification','environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'toolchains':{n:subprocess.check_output(c,text=True).strip() for n,c in [('stable',['rustc','-V']),('msrv',['rustc','+1.85.0','-V']),('cargo',['cargo','-V'])]},'limitations':['historical signed facts, not canonical chain/current ownership/full lifecycle/distributed atomicity','payment event facts do not authenticate unsigned settle intent declarations','full F8 fuzz, coverage, crash, package and API-review gates remain open','worktree all-target workspace attempt included an unbounded fuzz binary and was terminated; not counted as qualification']}
for name in ['crates/statechronicle-accumulator/src/event_tree.rs','crates/statechronicle-commit/src/roots.rs','crates/statechronicle-commit/Cargo.toml','crates/statechronicle-proof/src/settlement.rs','crates/statechronicle/tests/trade_history_proof.rs','Cargo.toml','Cargo.lock']:
 manifest['source_sha256'][name]=hashlib.sha256((source/name).read_bytes()).hexdigest()
for path in sorted((root/'target/settlement-history').glob('*.log')):
 dest=out/('worktree-'+path.name+'.gz');dest.write_bytes(gzip.compress(path.read_bytes(),mtime=0))
manifest['unqualified_worktree_logs']='Retained including failed payment/unborn check, strict-lint corrections, and terminated unbounded fuzz attempt. Only archived commands below qualify exact candidate.'
commands=[('parent-characterization',old,['cargo','test','-p','statechronicle','--test','trade_history_proof','history_and_proof_for_a_settled_two_tenant_trade','--locked','--offline']),('fmt',source,['cargo','fmt','--all','--','--check'])]
for tool,label in [('+1.85.0','msrv'),('', 'stable')]:
 cargo=['cargo']+([tool] if tool else [])
 commands += [(label+'-finite-workspace',source,cargo+['test','--workspace','--exclude','statechronicle-fuzz','--all-targets','--all-features','--locked','--offline']),(label+'-doctests',source,cargo+['test','--workspace','--exclude','statechronicle-fuzz','--doc','--all-features','--locked','--offline'])]
commands += [('clippy',source,['cargo','clippy','--workspace','--all-targets','--all-features','--locked','--offline','--','-D','warnings']),('doc',source,['cargo','doc','--workspace','--no-deps','--all-features','--locked','--offline']),('conformance',source,['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--locked','--offline'])]
for name,cwd,cmd in commands:
 local=dict(env)
 if name=='doc':local['RUSTDOCFLAGS']='-D warnings'
 if name=='conformance':local['CARGO_TARGET_DIR']=str(root/'ci/conformance/target')
 result=subprocess.run(cmd,cwd=cwd,env=local,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 dest=out/(name+'.log.gz');dest.write_bytes(gzip.compress(result.stdout,mtime=0))
 counts=[int(v) for v in re.findall(rb'test result: ok\. (\d+) passed',result.stdout)]
 manifest['commands'].append({'name':name,'source':parent if cwd==old else sha,'command':cmd,'exit_code':result.returncode,'passed_tests':sum(counts),'result_groups':len(counts),'RUSTDOCFLAGS':local.get('RUSTDOCFLAGS',''),'log_sha256':hashlib.sha256(dest.read_bytes()).hexdigest()})
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 print(name,result.returncode,sum(counts),flush=True)
 if result.returncode:
  print(result.stdout.decode()[-5000:],flush=True);raise SystemExit(result.returncode)
manifest['root_lock_unchanged_from_parent']=subprocess.check_output(['git','show',sha+':Cargo.lock'])==subprocess.check_output(['git','show',parent+':Cargo.lock'])
manifest['status']='passed'
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
