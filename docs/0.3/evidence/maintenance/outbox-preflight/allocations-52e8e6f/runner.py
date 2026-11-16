import pathlib,json,subprocess,gzip,re,hashlib,os
root=pathlib.Path.cwd();paired=root/'docs/0.3/evidence/maintenance/outbox-preflight/paired-52e8e6f';m=json.loads((paired/'manifest.json').read_text());out=paired.parent/'allocations-52e8e6f';out.mkdir(exist_ok=False)
def sha(b):return hashlib.sha256(b).hexdigest()
manifest=dict(candidate_commit=m['candidate_commit'],control_commit=m['control_commit'],checks=[],status='running',limitations=['Valgrind full-process counts include startup, fixture construction, warmups and repeated preflight, not only mutation actions.','Valgrind durations are not native latency; exact binaries match the native timing campaign.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
save()
try:
 for workload in ['outbox-preflight-0x0','outbox-preflight-1x65536']:
  for lane in ['control','candidate']:
   binary=pathlib.Path(m['lanes'][lane]['binary'])
   if sha(binary.read_bytes())!=m['lanes'][lane]['binary_sha256']:raise RuntimeError('binary drift')
   command=['valgrind','--error-exitcode=91','--leak-check=summary','--errors-for-leak-kinds=definite',str(binary),'--bench',workload]
   result=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=600)
   name=lane+'-'+workload
   for suffix,data in [('stdout',result.stdout),('stderr',result.stderr)]: (out/(name+'-'+suffix+'.log.gz')).write_bytes(gzip.compress(data,mtime=0))
   text=result.stderr.decode(errors='replace');match=re.search(r'total heap usage: ([\d,]+) allocs, ([\d,]+) frees, ([\d,]+) bytes allocated',text)
   values=json.loads(result.stdout)
   passed=result.returncode==0 and match is not None and 'ERROR SUMMARY: 0 errors' in text and len(values)==1 and values[0]['workload']==workload
   manifest['checks'].append(dict(lane=lane,workload=workload,command=command,exit_code=result.returncode,passed=passed,binary_sha256=sha(binary.read_bytes()),allocations=int(match[1].replace(',','')) if match else None,bytes_allocated=int(match[3].replace(',','')) if match else None));save();print(lane,workload,result.returncode,passed,flush=True)
   if not passed:raise RuntimeError(name+' failed')
 manifest['status']='passed'
except BaseException as e:manifest['status']='failed_or_interrupted';manifest['failure']=repr(e);raise
finally:
 manifest['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
