import pathlib,json,subprocess,os,gzip,hashlib,re
root=pathlib.Path.cwd();out=root/'docs/0.3/evidence/f1/settlement-history-e407af6';manifest=json.loads((out/'manifest.json').read_text());source=root/'target/settlement-history'/manifest['candidate_commit']/'candidate'
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/settlement-history/stable-isolated'))
manifest['stable_retry_reason']='Shared target directory produced missing newly added accumulator module while compiling the archived candidate. Retained failed run; fresh target retry checks actual source independently.'
commands=[('stable-isolated-finite-workspace',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--all-targets','--all-features','--locked','--offline']),('stable-isolated-doctests',['cargo','test','--workspace','--exclude','statechronicle-fuzz','--doc','--all-features','--locked','--offline']),('clippy-isolated',['cargo','clippy','--workspace','--all-targets','--all-features','--locked','--offline','--','-D','warnings']),('doc-isolated',['cargo','doc','--workspace','--no-deps','--all-features','--locked','--offline']),('conformance-isolated',['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--locked','--offline'])]
for name,cmd in commands:
 local=dict(env)
 if name=='doc-isolated':local['RUSTDOCFLAGS']='-D warnings'
 result=subprocess.run(cmd,cwd=source,env=local,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 dest=out/(name+'.log.gz');dest.write_bytes(gzip.compress(result.stdout,mtime=0))
 counts=[int(v) for v in re.findall(rb'test result: ok\. (\d+) passed',result.stdout)]
 manifest['commands'].append({'name':name,'source':manifest['candidate_commit'],'command':cmd,'exit_code':result.returncode,'passed_tests':sum(counts),'result_groups':len(counts),'environment':{k:local[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'RUSTDOCFLAGS':local.get('RUSTDOCFLAGS',''),'log_sha256':hashlib.sha256(dest.read_bytes()).hexdigest()})
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 print(name,result.returncode,sum(counts),flush=True)
 if result.returncode:
  print(result.stdout.decode()[-5000:],flush=True);raise SystemExit(result.returncode)
manifest['root_lock_unchanged_from_parent']=subprocess.check_output(['git','show',manifest['candidate_commit']+':Cargo.lock'])==subprocess.check_output(['git','show',manifest['parent_commit']+':Cargo.lock'])
manifest['status']='qualified_checks_passed_with_retained_failed_attempts'
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
