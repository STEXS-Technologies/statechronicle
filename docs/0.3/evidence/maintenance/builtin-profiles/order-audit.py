import pathlib,json,hashlib
root=pathlib.Path('docs/0.3/evidence/maintenance/builtin-profiles');report={}
for name in ['paired-20261116T125348Z-164088','paired-20261116T130401Z-565324','repeat-20261116T125631Z-278672','repeat-20261116T130653Z-711434']:
 path=root/name/'manifest.json';raw=path.read_bytes();m=json.loads(raw);orders={}
 for regime in ['default','extended']:
  bycycle={}
  for check in m['checks']:
   label=check.get('label','')
   if not label.startswith(regime+'-'):continue
   pieces=label.split('-');cycle=pieces[1];lane=pieces[2]
   if lane not in ['control','candidate']:raise RuntimeError('unknown lane')
   if lane not in bycycle.setdefault(cycle,[]):bycycle[cycle].append(lane)
  orders[regime]=bycycle
 report[name]={'manifest_sha256':hashlib.sha256(raw).hexdigest(),'observed_lane_orders':orders}
(root/'order-audit.py').write_bytes(pathlib.Path(__file__).read_bytes())
(root/'order-audit.json').write_text(json.dumps({'status':'audited','runner_sha256':hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),'runs':report,'finding':'Both paired campaigns used fixed control-first default and candidate-first extended orders; repeats use an explicit conditional alternation.'},indent=2)+'\n')
