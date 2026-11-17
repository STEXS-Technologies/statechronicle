"""Verify six fresh consumer builds and report their observed cost ranges."""
import gzip,hashlib,io,json,re,statistics,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest()
after=json.loads((out/'after-fuzz.json').read_text());repeat=json.loads((out/'repeat-build-costs.json').read_text())
assert after['status']==repeat['status']=='passed'
start=Path(after['matrix_path']).name.removeprefix('matrix-').split('-')[0]
original=subprocess.check_output(['git','rev-parse','v0.2.0^{commit}'],text=True).strip();candidate=after['candidate_commit']
observations=[]
for path in sorted((root/'docs/0.3/evidence/costs').glob('*/manifest.json')):
    if path.parent.name.split('-')[0]<start:continue
    m=json.loads(path.read_text());revision=subprocess.check_output(['git','rev-parse',m['revision']+'^{commit}'],text=True).strip()
    if revision not in [original,candidate]:continue
    assert m['status']=='passed' and 'release: 1.100.0' in m['toolchain']
    data=subprocess.check_output(['git','archive',m['revision']]);assert sha(data)==m['archive_sha256']
    source=root/'target/refactor-costs'/path.parent.name/'source'
    with tarfile.open(fileobj=io.BytesIO(data)) as handle:
        for f in handle:
            if f.isfile():assert sha((source/f.name).read_bytes())==sha(handle.extractfile(f).read()),f.name
    for name,h in m['consumer_sha256'].items():assert sha((root/'ci/cost-consumer'/name).read_bytes())==h
    assert sha((path.parent/'consumer.lock').read_bytes())==m['lock_sha256']
    logs={}
    for check in m['checks']:
        assert check['status']=='passed' and check['exit_code']==0
        raw=(path.parent/check['artifact']).read_bytes();assert sha(raw)==check['sha256'];logs[check['label']]=gzip.decompress(raw)
    raw=logs['cold-release-build'];packages=[json.loads(line) for line in raw.splitlines() if line.startswith(b'{')]
    artifacts=[a for a in packages if a.get('reason')=='compiler-artifact'];assert artifacts and all(not a['fresh'] for a in artifacts)
    assert len({a['package_id'] for a in artifacts})==m['compiled_packages']
    binary=root/'target/refactor-costs'/path.parent.name/'consumer/target/release/statechronicle-cost-consumer'
    assert sha(binary.read_bytes())==m['binary']['sha256'] and binary.stat().st_size==m['binary']['bytes']
    assert str(binary) in m['checks'][-1]['command']
    text=raw.decode();rss=int(re.search(r'Maximum resident set size \(kbytes\): (\d+)',text)[1]);clock=re.search(r'Elapsed \(wall clock\) time \(h:mm:ss or m:ss\): ([\d:.]+)',text)[1]
    seconds=0
    for value in clock.split(':'):seconds=seconds*60+float(value)
    sections=subprocess.check_output(['size','-A',str(binary)],text=True);text_bytes=int(re.search(r'^\.text\s+(\d+)',sections,re.M)[1]);artifact=path.parent/'binary-sections.log.gz';artifact.write_bytes(gzip.compress(sections.encode(),mtime=0))
    observations.append(dict(lane='original' if revision==original else 'candidate',revision=revision,path=str(path.parent.relative_to(root)),manifest_sha256=sha(path.read_bytes()),wall_seconds=seconds,max_rss_kib=rss,binary_bytes=m['binary']['bytes'],text_bytes=text_bytes,compiled_packages=m['compiled_packages'],binary_sections_sha256=sha(artifact.read_bytes())))
assert len(observations)==6,observations
assert [v['lane'] for v in observations]==['original','candidate','candidate','original','original','candidate']
summary={}
for lane in ['original','candidate']:
    values=[v for v in observations if v['lane']==lane];assert len(values)==3
    summary[lane]={metric:dict(values=[v[metric] for v in values],median=statistics.median(v[metric] for v in values),minimum=min(v[metric] for v in values),maximum=max(v[metric] for v in values)) for metric in ['wall_seconds','max_rss_kib','binary_bytes','text_bytes','compiled_packages']}
result=dict(status='passed',observations=observations,summary=summary,median_percent_deltas={metric:(summary['candidate'][metric]['median']/summary['original'][metric]['median']-1)*100 for metric in summary['original']},limitations=repeat['limitations']+['The tiny fixed facade binary is not full workflow binary size or runtime memory. Max RSS is the build process peak reported by /usr/bin/time.','Observed extrema of three samples are descriptive ranges, not confidence bounds or a universal regression threshold.'])
(out/'build-cost-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
