from pathlib import Path
import subprocess,os,json,hashlib,time
root=Path.cwd();base=root/'target/operation-registration/fuzz-smoke';base.mkdir(exist_ok=True)
targetdir=root/'target/operation-registration/instrumented'
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_NET_OFFLINE='true')
manifest={'candidate_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'compiler':subprocess.check_output(['rustc','+nightly','-Vv'],text=True),'cargo_fuzz':subprocess.check_output(['cargo','fuzz','--version'],text=True).strip(),'environment':{'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8','CARGO_NET_OFFLINE':'true'},'limitations':['Four modified harnesses only; bounded regression smoke, not the full F8 campaign.','Single process per target, fixed seed; actual runs may stop at time budget.'],'targets':[]}
for target in ['fuzz_portable','fuzz_trade_index_build','fuzz_commit_roots','fuzz_commit_extras']:
 d=base/target;d.mkdir(exist_ok=True);corpus=d/'corpus';corpus.mkdir(exist_ok=True)
 seeds={}
 for i,length in enumerate([0,1,2,3,32,33,34,48,64,65,66,96,97,127,128,255,256,257,1024,8191,8192,8193]):
  data=bytes(n%251 for n in range(length));p=corpus/f'seed-{i:02}';p.write_bytes(data);seeds[p.name]=hashlib.sha256(data).hexdigest()
 build=['cargo','+nightly','fuzz','build',target,'--target','x86_64-unknown-linux-gnu','--target-dir',str(targetdir),'--codegen-units','8']
 start=time.monotonic()
 with (d/'build.log').open('w') as out:rc=subprocess.run(build,env=env,stdout=out,stderr=subprocess.STDOUT).returncode
 result={'target':target,'source_sha256':hashlib.sha256((root/'fuzz/fuzz_targets'/f'{target}.rs').read_bytes()).hexdigest(),'seed_sha256':seeds,'build_command':build,'build_exit_code':rc,'build_seconds':time.monotonic()-start}
 if rc==0:
  binary=targetdir/'x86_64-unknown-linux-gnu/release'/target
  command=[str(binary),'-runs=4096','-max_total_time=30','-seed=20261116','-max_len=8193',f'-artifact_prefix={d}/',str(corpus)]
  result['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();result['run_command']=command
  with (d/'run.log').open('w') as out:result['run_exit_code']=subprocess.run(command,env=env,stdout=out,stderr=subprocess.STDOUT).returncode
 manifest['targets'].append(result);(base/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 print(target,result.get('build_exit_code'),result.get('run_exit_code'),flush=True)
 if rc or result.get('run_exit_code')!=0:raise SystemExit(1)
