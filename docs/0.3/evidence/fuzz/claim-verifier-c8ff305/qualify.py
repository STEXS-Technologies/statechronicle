import gzip,hashlib,json,os,pathlib,subprocess
root=pathlib.Path.cwd();out=pathlib.Path(__file__).resolve().parent;m=json.loads((out/'manifest.json').read_text())
sha=lambda b:hashlib.sha256(b).hexdigest()
def inputs():return {p:sha((root/p).read_bytes()) for p in m['source_sha256']}
if inputs()!=m['source_sha256']:raise RuntimeError('live source differs from immutable campaign')
qualification={'candidate_commit':m['candidate_commit'],'source_matches_campaign':True,'checks':[]}
for label,command in [('stable-fmt',['cargo','fmt','--all','--','--check']),('stable-clippy',['cargo','clippy','-p','statechronicle-fuzz','--bin','fuzz_claim_verifier','--locked','--offline','--','-D','warnings'])]:
 result=subprocess.run(command,env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8'),capture_output=True)
 raw=gzip.compress(result.stdout+result.stderr,mtime=0);name=label+'.log.gz';(out/name).write_bytes(raw)
 qualification['checks'].append({'label':label,'command':command,'exit_code':result.returncode,'artifact':name,'sha256':sha(raw)})
 if result.returncode:raise RuntimeError(label+' failed')
if inputs()!=m['source_sha256']:raise RuntimeError('source drift')
qualification['status']='passed';(out/'stable-qualification.json').write_text(json.dumps(qualification,indent=2)+'\n');print('stable formatting and strict target Clippy passed on matching source')
