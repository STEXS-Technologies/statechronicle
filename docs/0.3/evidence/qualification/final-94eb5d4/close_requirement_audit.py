"""Close manually reviewed roadmap clauses only after all final evidence rechecks."""
import argparse,gzip,hashlib,json,shutil,subprocess,tempfile
from pathlib import Path
cli=argparse.ArgumentParser();cli.add_argument('--accept-reviewed',action='store_true');args=cli.parse_args();assert args.accept_reviewed,'Explicit manual review disposition is required.'
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest()
final=json.loads((out/'final-gate-recheck.json').read_text());assert final['status']=='passed' and len(final['checks'])==17
for c in final['checks']:
 assert c['exit_code']==0 and sha((out/c['artifact']).read_bytes())==c['sha256']
 assert sha(Path(c['command'][1]).read_bytes())==c['verifier_sha256']
for name in ['after-fuzz.json','complete-performance.json','queued-verification.json','after-history.json','repeat-build-costs.json','performance-verification.json','history-performance-verification.json','portable-size-verification.json','metrics-verification.json','documentation-link-check.json']:
 assert json.loads((out/name).read_text())['status']=='passed',name
scopes=list(json.loads((out/'candidate-equivalence.json').read_text())['byte_identical_scopes'])
for rev in ['94eb5d4','0799afe','e880a2a']:subprocess.run(['git','diff','--exit-code',rev,'--',*scopes],check=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
reviews=['requirement-review.md','maintenance-review.md','api-review.md','finding-disposition.md','performance-review.md']
assert (out/'performance-review.md').read_text().splitlines()[0]=='# Accepted performance and build-cost disposition'
files=[('requirement-inventory.json','build_requirement_inventory.py',138),('supplemental-requirements.json','build_supplemental_inventory.py',84),('design-requirements.json','build_design_inventory.py',85)]
roadmap=root/'docs/0.3/ROADMAP.md';evidence={};inventories=[];count=0;exempt=[]
with tempfile.TemporaryDirectory(prefix='statechronicle-roadmap-audit-') as temporary:
 for filename,builder,expected in files:
  original=json.loads((out/filename).read_text());copy=Path(temporary)/builder;shutil.copyfile(out/builder,copy)
  subprocess.run(['python3',str(copy)],cwd=root,check=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  regenerated=json.loads((Path(temporary)/filename).read_text())
  assert [(r['id'],r['requirement']) for r in original['requirements']]==[(r['id'],r['requirement']) for r in regenerated['requirements']]
  assert len(original['requirements'])==expected and original['roadmap_sha256']==sha(roadmap.read_bytes())
  for row in original['requirements']:
   assert row.get('acceptance_basis'),row['id']
   if row['id']=='F8-9':
    assert row['status']=='separate_release_action';exempt.append(row['id']);continue
   assert row['status'] in ['pending_final_audit','reviewed_and_verified'],(row['id'],row['status'])
   assert row.get('review') and row.get('evidence')
   for name in row['evidence']:
    path=root/name if name.startswith(('docs/','ci/','crates/')) or name in ['CHANGELOG.md','README.md','TODO.md'] else out/name
    assert path.exists(),(row['id'],name)
    if path.is_file():evidence[str(path.relative_to(root))]=sha(path.read_bytes())
    else:
     # Named directories are navigation scope; their log/manifests are checked by the independent gates.
     evidence[str(path.relative_to(root))+'/']={'directory':True}
   row['status']='reviewed_and_verified';row['final_disposition']='Accepted within the literal clause, cited evidence, specific acceptance basis and documented consumer/backend/measurement limits. No stronger guarantee is inferred.';count+=1
  original['final_audit']=dict(status='reviewed_and_verified',roadmap_sha256=sha(roadmap.read_bytes()),scope='Qualification only; version/tag/publication separate.',reviewer='Codex implementing and reviewing external public consumers; not human maintainer signoff.')
  (out/filename).write_text(json.dumps(original,indent=2)+'\n');inventories.append(dict(path=filename,sha256=sha((out/filename).read_bytes()),rows=expected))
assert count==306 and exempt==['F8-9']
result=dict(status='passed',objective_scope='Full roadmap qualification, documentation/evidence commit and normal branch push. This record closes qualification; final Git commit/push verification is a separate final delivery action.',roadmap_sha256=sha(roadmap.read_bytes()),inventories=inventories,reviewed_clauses=count,separate_release_actions=exempt,manual_review_sha256={n:sha((out/n).read_bytes()) for n in reviews},final_gate_recheck_sha256=sha((out/'final-gate-recheck.json').read_bytes()),evidence=evidence,qualified_scopes=scopes,qualification_source_head='e880a2a36bb37ca5d2a52e68721c5b9e4ec40e9b',limitations=['307 literal inventory rows overlap; they are not 307 independent test results.','Original policy failure, timeout/interruption attempts and invalid/ineffective controls remain unqualified and retained.','No registry release, physical backend certification, arbitrary downstream compatibility, human signoff or gap-free claim.'])
(out/'requirement-audit.json').write_text(json.dumps(result,indent=2)+'\n');print('Qualification audit accepted 306 reviewed clauses; F8-9 still requires separate release authorization.')
