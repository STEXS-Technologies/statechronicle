"""Verify signed-history latency/allocation logs and exact consumer provenance."""
import gzip,hashlib,io,json,math,re,statistics,subprocess,tarfile,tomllib
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;base=out/'history-performance';sha=lambda b:hashlib.sha256(b).hexdigest();m=json.loads((base/'manifest.json').read_text())
assert m['status']=='passed';logs={}
build=json.loads((base/'build-manifest.json').read_text());assert build['status']=='built'
assert sha((base/'build-manifest.json').read_bytes())==m['cpu_configuration_review']['build_manifest_sha256']
assert m['cpu']==m['cpu_configuration_review']['selected_cpu']==6
assert m['cpu_configuration_review']['utc']<=m['conditions_before']['utc']
assert all(c['label'] in ['control-resolve','control-build','candidate-resolve','candidate-build'] for c in build['checks'])
smoke=json.loads((out/'history-smoke.json').read_text());assert smoke['status']=='passed' and smoke['build_manifest_sha256']==sha((base/'build-manifest.json').read_bytes()) and len(smoke['checks'])==4
for c in smoke['checks']:
    assert c['exit_code']==0 and sha((out/c['artifact']).read_bytes())==c['sha256']
    assert c['extra_environment']==dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS='1')
    assert c['binary_sha256'] in [item['binary_sha256'] for item in m['lanes'].values()]
for lane,item in build['lanes'].items():assert item==m['lanes'][lane]

sources={}
if 'allocation_sources' in m:
    expected_sources=dict(retained_control='history-performance/manifest-failed-900-seconds.json',resumed_control='history-control-resume/manifest.json',candidate='history-candidate-allocations/manifest.json')
    assert set(m['allocation_sources'])==set(expected_sources)
    for key,name in expected_sources.items():
        reference=m['allocation_sources'][key];assert reference['path']==name and sha((out/name).read_bytes())==reference['sha256'];sources[key]=json.loads((out/name).read_text())
    assert sources['retained_control']['status']=='failed' and 'TimeoutExpired' in sources['retained_control']['error']
    assert sources['resumed_control']['status']==sources['candidate']['status']=='passed'
    assert m['lanes']==sources['retained_control']['lanes'] and m['fixture_sha256']==sources['retained_control']['fixture_sha256']
    assert [c for c in m['checks'] if not c['label'].startswith('allocation-')]==[c for c in sources['retained_control']['checks'] if not c['label'].startswith('allocation-')]
    integration=m['allocation_integration'];assert sha((out/'complete_history_parallel.py').read_bytes())==integration['runner_sha256']
    assert sha((out/'after-history-failed-900-seconds.json').read_bytes())==integration['failed_orchestrator_sha256']
    for script,report in [('verify_history_control_resume.py','history-control-allocation-verification.json'),('verify_history_candidate_allocations.py','history-candidate-allocation-verification.json')]:
        result=subprocess.run(['python3',str(out/script)],cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);assert result.returncode==0,result.stdout.decode(errors='replace')
        assert json.loads((out/report).read_text())['status']=='passed' and sha((out/report).read_bytes())==integration['independent_reports'][report]
assert len(m['checks'])==len({c['label'] for c in m['checks']})
for c in m['checks']:
    assert c['exit_code']==0
    if c['label'].startswith('allocation-'):
        if sources:
            origin=c['allocation_origin'];assert origin in sources
            original=next(v for v in sources[origin]['checks'] if v['label']==c['label'])
            assert {k:v for k,v in c.items() if k not in ['artifact','allocation_origin']}=={k:v for k,v in original.items() if k!='artifact'}
            assert c['artifact']==(original['artifact'] if origin=='retained_control' else origin+'-'+original['artifact'])
            source=out/m['allocation_sources'][origin]['path'];assert sha((source.parent/original['artifact']).read_bytes())==c['sha256']
            assert (origin=='candidate')==c['label'].startswith('allocation-candidate-')
        else:assert c['command'][:3]==['taskset','-c',str(m['cpu'])]
    elif c['label'].split('-')[0].isdigit():assert c['command'][:3]==['taskset','-c',str(m['cpu'])]
    data=(base/c['artifact']).read_bytes();assert sha(data)==c['sha256'];logs[c['label']]=gzip.decompress(data)
registry_features={};registry_packages={}
for lane,item in m['lanes'].items():
    data=subprocess.check_output(['git','archive',item['revision'],'--',*item['archive_paths']]);assert sha(data)==item['archive_sha256']
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:files={f.name:tar.extractfile(f).read() for f in tar if f.isfile()}
    assert {p:sha(b) for p,b in files.items()}==item['source_sha256']
    binary=base/('bench-'+lane.ljust(9,'_'));assert sha(binary.read_bytes())==item['binary_sha256']
    metadata=[json.loads(line) for line in logs[lane+'-resolve'].splitlines() if line.startswith(b'{')];assert len(metadata)==1;consumer=Path(metadata[0]['workspace_root'])
    meta=metadata[0];packages={v['id']:v for v in meta['packages']};registry_features[lane]=sorted((packages[v['id']]['name'],packages[v['id']]['version'],packages[v['id']]['source'],tuple(sorted(v['features']))) for v in meta['resolve']['nodes'] if packages[v['id']]['source'])
    registry_packages[lane]=sorted((v['name'],v['version'],v['source'],v['checksum']) for v in tomllib.loads((consumer/'Cargo.lock').read_text())['package'] if v.get('source'))
    assert {str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()}==item['consumer_sha256']
    assert sha((consumer/'Cargo.lock').read_bytes())==item['consumer_lock_sha256']
    assert (consumer/'src/main.rs').read_bytes()==b'#![allow(dead_code, unused_variables, unused_imports)]\n'+(base/'benchmark.rs').read_bytes()
    for path,h in m['fixture_sha256'].items():assert sha(subprocess.check_output(['git','show',m['candidate_commit']+':'+path]))==h
    for crate in (root/'crates').iterdir():
        if (crate/'Cargo.toml').exists():assert ('Compiling '+crate.name+' v').encode() in logs[lane+'-build']
assert registry_features['control']==registry_features['candidate'] and registry_packages['control']==registry_packages['candidate']
summary=json.loads((base/'summary.json').read_text());assert len(summary)==7
names={'control',*[f'{prefix}_{n}' for n in [1,32,256] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]};assert {r['workload'] for r in summary}==names
for record in summary:
    name=record['workload'];expected_n=1000 if name=='control' else 100 if name.endswith('_1') else 31
    for lane,values in record['observations'].items():
        assert len(values)==7
        for cycle,value in enumerate(values):
            raw=json.loads(logs[str(cycle)+'-'+lane]);assert raw['fixture_digests']==m['observed_fixture_digests'] and len(raw['fixture_digests'])==3
            assert [v for v in raw['workloads'] if v['workload']==name]==[value]
            samples=value['samples_ns'];assert len(samples)==value['sample_count']==expected_n and samples==sorted(samples) and all(v>=0 for v in samples)
            assert value['median_ns']==samples[len(samples)//2]
            for percentile in [95,99]:assert value[f'p{percentile}_ns']==samples[math.ceil(len(samples)*percentile/100)-1]
    for metric,deltas in record['paired_percent_deltas'].items():
        assert deltas==[(b[metric]/a[metric]-1)*100 for a,b in zip(record['observations']['control'],record['observations']['candidate'])]
        assert statistics.median(deltas)==record['median_paired_percent_delta'][metric]
allocation_names=[('control',n) for n in [1000,100,10]]+[(f'{prefix}_{n}',count) for n,count in [(1,1000),(32,100),(256,10)] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]
expected_allocation_labels={f'allocation-{lane}-{name}-{count}' for lane in ['control','candidate'] for name,count in allocation_names}
assert {c['label'] for c in m['checks'] if c['label'].startswith('allocation-')}==expected_allocation_labels
assert set(m['allocations'])=={'control','candidate'}
assert all(set(values)=={name+'@'+str(count) for name,count in allocation_names} for values in m['allocations'].values())
for lane,values in m['allocations'].items():
    assert len(values)==9
    for name,totals in values.items():
        workload,count=name.split('@');count=int(count);assert count==totals['actions']
        c=next(c for c in m['checks'] if c['label']==f'allocation-{lane}-{workload}-{count}')
        assert c['extra_environment']==dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count))
        raw=logs[c['label']];assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
        match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
        actual=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));assert all(totals[k]==v for k,v in actual.items())
        control=values['control@'+str(count)];assert totals['control_subtracted_per_action']=={key:(totals[key]-control[key])/count for key in ['allocations','bytes']}
after=json.loads((out/'after-history.json').read_text());assert after['status']=='passed'
for c in after['checks']:assert c['exit_code']==0 and sha((out/c['artifact']).read_bytes())==c['sha256']
result=dict(status='passed',manifest_sha256=sha((base/'manifest.json').read_bytes()),workloads=7,cycles=7,allocation_probes=18,verified_fixture_digests=m['observed_fixture_digests'],matched_registry_features=registry_features['control'],matched_registry_packages=registry_packages['control'],limitations=m['limitations']+m['allocation_limitations'])
(out/'history-performance-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
