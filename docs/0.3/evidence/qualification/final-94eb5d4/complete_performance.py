"""Complete all-path individual latency and allocation evidence after cold builds."""
import gzip, hashlib, json, os, pathlib, re, subprocess, time
root=pathlib.Path.cwd(); out=pathlib.Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
m=dict(pid=os.getpid(),status='waiting_for_build_costs',checks=[])
def save():(out/'complete-performance.json').write_text(json.dumps(m,indent=2)+'\n')
def execute(label,command):
    m['stage']=label;save();start=time.monotonic();p=out/(label+'.live.log')
    with p.open('wb') as stream:r=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT)
    raw=p.read_bytes();data=gzip.compress(raw,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(data);p.unlink()
    m['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data)));save()
    assert r.returncode==0,(label,r.returncode)
    return raw.decode(errors='replace')
save()
try:
    while True:
        d=json.loads((out/'repeat-build-costs.json').read_text())
        if d['status'] not in ['waiting_for_timings','running']:
            assert d['status']=='passed',d
            break
        assert pathlib.Path('/proc/'+str(d['pid'])).exists(),'cold-build orchestrator stopped'
        time.sleep(15)
    m['status']='running';save()
    matrix=json.loads((out/'after-fuzz.json').read_text())['matrix_path']
    code=(out/'per-request_final.py').read_text()
    code=code.replace("('per-request-'+stamp)","('all-paths-'+stamp)")
    old="workloads=['control','authenticated_prepare_including_adapter_setup','authenticated_two_intent_batch_to_refusing_sink_including_setup','two_intent_planning_including_adapter_setup','cross_tenant_trade_plan_including_seed_setup']"
    assert old in code
    code=code.replace(old,"workloads=[v['workload'] for v in json.loads((base/'summary.json').read_text()) if v['regime']=='default'];assert len(workloads)==22")
    code=code.replace("'1' if name=='control' else '20'","'1'")
    code=code.replace("assert values[0]['sample_count']==(1240 if name=='cross_tenant_trade_plan_including_seed_setup' else 3100)","assert values[0]['sample_count']>=62 and values[0]['sample_count']%31==0")
    code=code.replace('31 times 100 actions for single/refusal/planning, 31 times 40 for trade; 10 warmups in every process.','31 times the frozen default workload iteration count; actual sample counts retained; 10 warmups per process. Separate heavy-path evidence uses larger samples.')
    old='    for _ in 0..10 { action(); }'
    assert old in code
    code=code.replace(old,'''    if std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok().as_deref()==Some("1") {
        for _ in 0..1000 { action(); }
        std::process::exit(0);
    }
'''+old)
    old="code=code[:start]+function+code[end:];"
    assert old in code
    code=code.replace(old,old+'''code=code.replace('let only = std::env::args().nth(1);', 'let only = std::env::args().nth(1).map(|mut s| { let n = s.trim_end_matches(\'_\').len(); s.truncate(n); s });');''')
    # The replacement above is generated Python: use repr to preserve embedded quotes.
    code=code.replace("'let only = std::env::args().nth(1).map(|mut s| { let n = s.trim_end_matches('_').len(); s.truncate(n); s });'",repr("let only = std::env::args().nth(1).map(|mut s| { let n = s.trim_end_matches('_').len(); s.truncate(n); s });"))
    driver=out/'all_paths_final.py';driver.write_text(code);compile(code,str(driver),'exec')
    result=execute('all-paths',['python3',str(driver)])
    path=next(s.strip() for s in reversed(result.splitlines()) if s.startswith('docs/0.3/evidence/performance/all-paths-'))
    m['latency_path']=path;save();base=root/path;prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='passed'
    stamp=base.name.removeprefix('all-paths-');run=root/'target/per-request'/stamp
    allocation=base/'allocations.json';a=dict(status='running',actions_per_probe=1000,valgrind=subprocess.check_output(['valgrind','--version'],text=True).strip(),source_manifest_sha256=sha((base/'manifest.json').read_bytes()),checks=[],results={},limitations=['Full process totals include fixed fixture construction. Control subtraction describes only these fixtures.','Selected workload executes 1000 actions and exits before timing; process exit skips fixture destruction, so frees are not leak qualification.','Equal-length executable paths and padded arguments control startup argument allocation. Timing never uses Valgrind.'])
    def save_a():allocation.write_text(json.dumps(a,indent=2)+'\n')
    save_a();names=[v['workload'] for v in json.loads((base/'summary.json').read_text())]
    assert len(names)==22 and max(map(len,names))<128
    for index,(lane,old) in enumerate(prior['lanes'].items()):
        binary=run/f'lane-{index}-bench';assert sha(binary.read_bytes())==old['binary_sha256'];a['results'][lane]={}
        for name in names:
            command=['taskset','-c',str(prior['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
            start=time.monotonic();r=subprocess.run(command,cwd=root,env=dict(os.environ,STATECHRONICLE_BENCH_MULTIPLIER='1',STATECHRONICLE_ALLOCATION_PROBE='1'),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180)
            data=gzip.compress(r.stdout,mtime=0);artifact='allocation-'+lane+'-'+name+'.log.gz';(base/artifact).write_bytes(data)
            a['checks'].append(dict(command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data)));save_a();assert r.returncode==0
            match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',r.stdout);assert match
            assert re.search(rb'ERROR SUMMARY: 0 errors',r.stdout)
            a['results'][lane][name]=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));save_a()
        control=a['results'][lane]['control']
        for name,totals in a['results'][lane].items():totals['control_subtracted_per_action']={key:(totals[key]-control[key])/1000 for key in ['allocations','bytes']}
        for name in names:
            if name in ['native_validate_reduce','portable_validate_reduce_hash']:
                assert all(a['results'][lane][name][key]==control[key] for key in ['allocations','bytes'])
        assert sha(binary.read_bytes())==old['binary_sha256'];save_a()
    a['status']='passed';save_a();m['allocation_manifest']=str(allocation.relative_to(root));m['allocation_manifest_sha256']=sha(allocation.read_bytes());m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:save()
print('All-path timing and allocation campaign passed.',flush=True)
