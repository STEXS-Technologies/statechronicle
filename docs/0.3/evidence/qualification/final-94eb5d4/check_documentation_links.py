"""Check local file destinations in changed documentation, excluding URL/anchors."""
import hashlib,json,re,subprocess,urllib.parse
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent
tracked=subprocess.check_output(['git','diff','--name-only'],text=True).splitlines()
new=subprocess.check_output(['git','ls-files','--others','--exclude-standard','docs/0.3'],text=True).splitlines()
paths=sorted({p for p in tracked+new if p.endswith('.md') and not p.endswith('/SESSION.md')});checks=[]
for name in paths:
 p=root/name
 for match in re.finditer(r'\[[^\]\n]+\]\(([^)\n]+)\)',p.read_text()):
  target=match.group(1).strip().strip('<>')
  if re.match(r'^[A-Za-z][A-Za-z0-9+.-]*:',target) or target.startswith('#'):continue
  target=urllib.parse.unquote(target.split('#')[0]);target=target.split(' "')[0]
  dest=(p.parent/target).resolve() if not target.startswith('/') else root/target.lstrip('/')
  checks.append(dict(source=name,target=target,exists=dest.exists()))
result=dict(status='passed' if all(c['exists'] for c in checks) else 'failed',checked_files={n:hashlib.sha256((root/n).read_bytes()).hexdigest() for n in paths},checks=checks,limitations=['Checks local file destinations only; remote URLs and heading anchors are excluded.','Historical checkpoint prose is retained with a link to current qualification.'])
(out/'documentation-link-check.json').write_text(json.dumps(result,indent=2)+'\n')
for c in checks:
 if not c['exists']:print(c)
assert result['status']=='passed'
print(len(paths),'documents;',len(checks),'local file links passed')
