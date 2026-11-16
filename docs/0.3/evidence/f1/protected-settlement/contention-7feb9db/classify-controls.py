import gzip,json,pathlib,hashlib
out=pathlib.Path('docs/0.3/evidence/f1/protected-settlement/contention-7feb9db');manifest=json.loads((out/'manifest.json').read_text())
classifications={
 'accept-stale-authority':('extra buyer debited after authoritative guards bypassed', ['assertion `left == right` failed','left: String("900")','right: String("1000")']),
 'leak-payment-projection':('rollback snapshot contains leaked payment projection', ['assertion `left == right` failed','StateProjection','mantissa: 900']),
 'acknowledge-lost-response':('fixture falsely acknowledges a modeled lost response', ['Result::unwrap_err()','Ok','Committed(CommittedBatch']),
}
for check in manifest['checks']:
 lane=check['lane']
 if lane in classifications:
  label,needles=classifications[lane];log=gzip.decompress((out/check['artifact']).read_bytes()).decode()
  if not all(needle in log for needle in needles):raise RuntimeError('unexpected failure for '+lane)
  check['failure_classification']=label;check['failure_markers_verified']=needles
manifest['failure_classification_runner_sha256']=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
(out/'classify-controls.py').write_bytes(pathlib.Path(__file__).read_bytes())
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
