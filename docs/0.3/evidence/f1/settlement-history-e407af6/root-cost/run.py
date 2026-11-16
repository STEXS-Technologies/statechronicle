import os,pathlib,subprocess,hashlib,json,gzip,statistics,re,shutil
root=pathlib.Path.cwd();out=root/'docs/0.3/evidence/f1/settlement-history-e407af6/root-cost';out.mkdir(parents=True,exist_ok=True);base=root/'target/settlement-history/root-bench';source=root/'target/settlement-history/e407af6d5ac55b08f3659235fd6a3fe8a2b04218';bench=(base/'main.rs').read_bytes();(out/'main.rs').write_bytes(bench)
manifest={'baseline_commit':'316099b2e003425f7b386edc82834d8c181737e7','candidate_commit':'e407af6d5ac55b08f3659235fd6a3fe8a2b04218','benchmark_sha256':hashlib.sha256(bench).hexdigest(),'cpu':min(os.sched_getaffinity(0)),'toolchain':subprocess.check_output(['rustc','-Vv'],text=True),'environment':{'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8'},'builds':[],'runs':[],'allocations':{},'limitations':['Shared host, seven alternating paired processes; not final isolated F8 performance','31 sample averages; tails are not individual-request p95','Measures only existing event-root function on fixed synthetic events, not new settlement verification or full mutation','Same compiler, manifests and third-party versions; control is immediate parent, not original release']}
manifest['runner_sha256']=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest();(out/'run.py').write_bytes(pathlib.Path(__file__).read_bytes());binaries={};results={'baseline':[],'candidate':[]}
def save(): (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
def log(name,data):
 dest=out/(name+'.log.gz');dest.write_bytes(gzip.compress(data,mtime=0));return hashlib.sha256(dest.read_bytes()).hexdigest()
for lane,dirname in [('baseline','parent'),('candidate','candidate')]:
 crate=base/lane;(crate/'src').mkdir(parents=True,exist_ok=True);(crate/'src/main.rs').write_bytes(bench)
 (crate/'Cargo.toml').write_text('[package]\nname="event-root-cost"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="'+str(source/dirname/'crates/statechronicle')+'"}\n')
 shutil.copyfile(source/dirname/'Cargo.lock',crate/'Cargo.lock')
 env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(crate/'target'))
 cmd=['cargo','build','--release','--manifest-path',str(crate/'Cargo.toml'),'--offline']
 result=subprocess.run(cmd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 record={'lane':lane,'command':cmd,'exit_code':result.returncode,'log_sha256':log(lane+'-build',result.stdout),'resolved_lock_sha256':hashlib.sha256((crate/'Cargo.lock').read_bytes()).hexdigest()};manifest['builds'].append(record);save();print(lane,'build',result.returncode,flush=True)
 if result.returncode:raise SystemExit(result.stdout.decode())
 (out/(lane+'.lock')).write_bytes((crate/'Cargo.lock').read_bytes());binary=crate/'target/release/event-root-cost';binaries[lane]=binary;record['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
for pair in range(7):
 for lane in (['baseline','candidate'] if pair%2==0 else ['candidate','baseline']):
  cmd=['taskset','-c',str(manifest['cpu']),str(binaries[lane])];r=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);manifest['runs'].append({'pair':pair,'lane':lane,'command':cmd,'exit_code':r.returncode,'log_sha256':log(f'pair-{pair}-{lane}',r.stdout)});save();assert r.returncode==0;results[lane].append(json.loads(r.stdout))
 print('pair',pair,'passed',flush=True)
summary=[]
for index in range(3):
 b=[v[index] for v in results['baseline']];c=[v[index] for v in results['candidate']];assert all(v['root']==b[0]['root'] for v in b+c)
 summary.append({'events':b[0]['events'],'root':b[0]['root'],'paired_median_percent_deltas':[(y['median_ns']/x['median_ns']-1)*100 for x,y in zip(b,c)],'median_paired_percent_delta':statistics.median((y['median_ns']/x['median_ns']-1)*100 for x,y in zip(b,c)),'median_paired_absolute_ns_delta':statistics.median(y['median_ns']-x['median_ns'] for x,y in zip(b,c)),'median_paired_p95_percent_delta':statistics.median((y['p95_ns']/x['p95_ns']-1)*100 for x,y in zip(b,c))})
for lane,binary in binaries.items():
 for count in [0,1,3,256]:
  cmd=['valgrind','--tool=memcheck','--leak-check=no',str(binary),str(count)];r=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);digest=log(f'alloc-{lane}-{count}',r.stdout);assert r.returncode==0;match=re.search(rb'total heap usage: ([\d,]+) allocs, ([\d,]+) frees, ([\d,]+) bytes allocated',r.stdout);assert match
  manifest['allocations'][f'{lane}-{count}']={'command':cmd,'exit_code':r.returncode,'log_sha256':digest,'allocations':int(match[1].replace(b',',b'')),'bytes':int(match[3].replace(b',',b''))};save()
for count in [1,3,256]:
 for metric in ['allocations','bytes']:
  b=manifest['allocations'][f'baseline-{count}'][metric]-manifest['allocations']['baseline-0'][metric];c=manifest['allocations'][f'candidate-{count}'][metric]-manifest['allocations']['candidate-0'][metric];assert b==c,(count,metric,b,c)
(out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');manifest['summary_sha256']=hashlib.sha256((out/'summary.json').read_bytes()).hexdigest();manifest['status']='passed';save();print(summary,flush=True)
