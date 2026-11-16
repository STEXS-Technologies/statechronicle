from pathlib import Path
import subprocess,concurrent.futures,time,json,hashlib,os,shutil
root=Path.cwd();base=root/'target/release-audit/fuzz-smoke';base.mkdir(exist_ok=True)
targets=subprocess.check_output(['cargo','+nightly','fuzz','list'],text=True).splitlines()
assert len(targets)==25
binary_dir=root/'target/release-audit/fuzz-build/x86_64-unknown-linux-gnu/release'
def run(name):
 binary=binary_dir/name;corpus=base/name;corpus.mkdir(exist_ok=True)
 source=root/'fuzz/corpus'/name
 if source.is_dir():shutil.copytree(source,corpus,dirs_exist_ok=True)
 cmd=[str(binary),'-runs=1000','-max_total_time=5','-max_len=4096','-rss_limit_mb=2048','-seed=7331','-artifact_prefix='+str(base/(name+'-')),str(corpus)]
 start=time.monotonic();r=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,env=dict(os.environ,ASAN_OPTIONS='detect_leaks=0'));log=base/(name+'.log');log.write_bytes(r.stdout)
 item={'target':name,'command':cmd,'exit_code':r.returncode,'seconds':time.monotonic()-start,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'log':log.name,'log_sha256':hashlib.sha256(log.read_bytes()).hexdigest(),'corpus_files':len(list(corpus.iterdir()))};print(name,r.returncode,flush=True);return item
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:results=list(pool.map(run,targets))
manifest={'library_and_fuzz_commit':'f7cd78941febb4e3157fa3047c7f74c3ae68e380','observed_head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'nightly':subprocess.check_output(['rustc','+nightly','-Vv'],text=True),'build_command':['cargo','+nightly','fuzz','build','--target','x86_64-unknown-linux-gnu'],'runs':results,'limitations':['Bounded ASAN instrumented smoke, not full fuzz qualification.','Existing local corpora copied to ignored run directory and not mutated in place.','Max-time may stop before run limit; retained logs contain actual counts.']}
(base/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');assert all(r['exit_code']==0 for r in results)
