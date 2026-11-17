"""Verify the original seven control probes and two full-scope timeout retries."""
import gzip,hashlib,json,re
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;base=out/'history-performance';dest=out/'history-control-resume';sha=lambda b:hashlib.sha256(b).hexdigest()
m=json.loads((dest/'manifest.json').read_text());failed=base/'manifest-failed-900-seconds.json';old=json.loads(failed.read_text());after=out/'after-history-failed-900-seconds.json';outer=json.loads(after.read_text())
assert m['status']=='passed' and m['timeout_seconds']==3600
assert old['status']=='failed' and 'TimeoutExpired' in old['error'] and ', 900)' in old['error']
assert sha(failed.read_bytes())==m['failed_manifest_sha256'] and sha(after.read_bytes())==m['failed_orchestrator_sha256']
assert outer['status']=='failed' and outer['checks'][-1]['exit_code']==1
for c in outer['checks']:
 data=(out/c['artifact']).read_bytes();assert sha(data)==c['sha256']
assert b'TimeoutExpired' in gzip.decompress((out/outer['checks'][-1]['artifact']).read_bytes())
assert sha((base/'summary.json').read_bytes())==m['timing_summary_sha256']
assert sha((dest/'runner.py').read_bytes())==m['runner_sha256']
assert (dest/'runner.py').read_bytes()==(out/'resume_history_control.py').read_bytes()
binary=base/'bench-control__';assert sha(binary.read_bytes())==m['binary_sha256']==old['lanes']['control']['binary_sha256']
expected=[('control',n) for n in [1000,100,10]]+[(f'{prefix}_{n}',count) for n,count in [(1,1000),(32,100),(256,10)] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]
labels=[f'allocation-control-{name}-{count}' for name,count in expected]
assert [c['label'] for c in m['prior_positive_checks']]==labels[:7]
assert m['prior_positive_checks']==[c for c in old['checks'] if c['label'].startswith('allocation-control-')]
assert [c['label'] for c in m['checks']]==labels[7:]
assert m['cpu_by_label']=={labels[7]:6,labels[8]:8}
assert set(m['topology'])=={'6','8'} and len({(v['physical_package_id'],v['core_id']) for v in m['topology'].values()})==2
assert len(m['allocations'])==9;totals={};logs={}
for index,((name,count),c) in enumerate(zip(expected,m['prior_positive_checks']+m['checks'])):
 cpu=6 if index<7 else m['cpu_by_label'][c['label']]
 assert c['exit_code']==0 and c.get('timed_out',False) is False
 assert c['command']==['taskset','-c',str(cpu),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
 assert c['extra_environment']==dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count))
 folder=base if index<7 else dest;data=(folder/c['artifact']).read_bytes();assert sha(data)==c['sha256'];raw=gzip.decompress(data)
 assert re.search(rb'ERROR SUMMARY: 0 errors from 0 contexts',raw)
 assert b'Command: '+str(binary).encode()+b' '+name.ljust(128,'_').encode() in raw
 match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
 actual=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));actual['actions']=count;totals[name+'@'+str(count)]=actual;logs[c['label']]=c['sha256']
for name,actual in totals.items():
 count=actual['actions'];control=totals['control@'+str(count)];actual['control_subtracted_per_action']={key:(actual[key]-control[key])/count for key in ['allocations','bytes']}
 assert actual==m['allocations'][name] and all(v>=0 for v in actual['control_subtracted_per_action'].values())
result=dict(status='passed',manifest_sha256=sha((dest/'manifest.json').read_bytes()),failed_manifest_sha256=m['failed_manifest_sha256'],binary_sha256=m['binary_sha256'],retained_probes=7,resumed_probes=2,raw_log_sha256=logs,allocations=totals,scope='Full control allocation scope; original900secondattemptremainsfailed. Candidate/source/timing integration remains separate.')
(out/'history-control-allocation-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
