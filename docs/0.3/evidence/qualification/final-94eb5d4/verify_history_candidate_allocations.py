"""Independently verify all nine parallel candidate history allocation probes."""
import gzip,hashlib,json,re
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;base=out/'history-performance';dest=out/'history-candidate-allocations'
sha=lambda b:hashlib.sha256(b).hexdigest()
m=json.loads((dest/'manifest.json').read_text());build=json.loads((base/'build-manifest.json').read_text())
assert m['status']=='passed' and m['lane']=='candidate' and m['cpu']==2 and m['timeout_seconds']==3600
assert sha((base/'build-manifest.json').read_bytes())==m['build_manifest_sha256']
assert sha((base/'summary.json').read_bytes())==m['summary_sha256']
assert sha((dest/'runner.py').read_bytes())==m['driver_sha256']
assert (dest/'runner.py').read_bytes()==(out/'history_candidate_allocations.py').read_bytes()
binary=base/'bench-candidate';assert sha(binary.read_bytes())==m['binary_sha256']==build['lanes']['candidate']['binary_sha256']
expected=[('control',n) for n in [1000,100,10]]
expected += [(f'{prefix}_{n}',count) for n,count in [(1,1000),(32,100),(256,10)] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]
assert len(m['checks'])==len(m['allocations'])==9
cpu_by_label={f'allocation-candidate-{name}-{count}':2 for name,count in expected}
if 'parallel_handoff' in m:
 h=m['parallel_handoff'];interrupted=json.loads((dest/'manifest-interrupted-for-parallel-handoff.json').read_text());review=json.loads((dest/'parallel-handoff-review.json').read_text())
 assert sha((dest/'parallel-handoff-review.json').read_bytes())==h['review_sha256']
 assert sha((dest/'manifest-interrupted-for-parallel-handoff.json').read_bytes())==h['interrupted_manifest_sha256']==review['manifest_interrupted_sha256']
 assert interrupted['status']=='failed' and interrupted['checks'][-1]['exit_code']==-15
 assert review['status']=='intentionally_interrupted_and_preserved' and review['interrupted_exit_code']==-15
 assert sha((dest/'runner-parallel.py').read_bytes())==h['parallel_runner_sha256']
 assert (dest/'runner-parallel.py').read_bytes()==(out/'history_candidate_parallel.py').read_bytes()
 assert m['checks'][:4]==[c for c in interrupted['checks'] if c['exit_code']==0]
 stopped=interrupted['checks'][-1];assert sha((dest/stopped['artifact']).read_bytes())==stopped['sha256']
 assert [c['label'] for c in m['checks'][:4]]==review['completed_positive_probes']
 expected_cpus=dict(zip([f'allocation-candidate-{name}-{count}' for name,count in expected[4:]],[2,3,4,5,7]))
 assert h['cpu_by_label']==expected_cpus
 assert len({(v['physical_package_id'],v['core_id']) for v in h['topology'].values()})==5
 assert set(h['topology'])=={str(v) for v in expected_cpus.values()}
 cpu_by_label.update(expected_cpus)
assert [c['label'] for c in m['checks']]==[f'allocation-candidate-{name}-{count}' for name,count in expected]
totals={};logs={}
for (name,count),c in zip(expected,m['checks']):
 assert c['exit_code']==0 and c['timed_out'] is False and 0<c['seconds']<=m['timeout_seconds']+10
 assert c['command']==['taskset','-c',str(cpu_by_label[c['label']]),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
 assert c['extra_environment']==dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count))
 data=(dest/c['artifact']).read_bytes();assert sha(data)==c['sha256'];raw=gzip.decompress(data)
 assert re.search(rb'ERROR SUMMARY: 0 errors from 0 contexts',raw)
 assert b'Command: '+str(binary).encode()+b' '+name.ljust(128,'_').encode() in raw
 match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
 actual=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));actual['actions']=count
 totals[name+'@'+str(count)]=actual;logs[c['label']]=c['sha256']
for name,actual in totals.items():
 count=actual['actions'];control=totals['control@'+str(count)]
 actual['control_subtracted_per_action']={key:(actual[key]-control[key])/count for key in ['allocations','bytes']}
 assert actual==m['allocations'][name]
 assert all(v>=0 for v in actual['control_subtracted_per_action'].values())
result=dict(status='passed',manifest_sha256=sha((dest/'manifest.json').read_bytes()),binary_sha256=m['binary_sha256'],probes=9,cpu_by_label=cpu_by_label,raw_log_sha256=logs,allocations=totals,scope='Full independent candidate allocation lane only. Control-lane completion, integrated history provenance and aggregate qualification remain separate.')
(out/'history-candidate-allocation-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
