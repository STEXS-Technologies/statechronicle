import copy,gzip,json
from pathlib import Path
r=Path('docs/0.3/evidence/api')
def keyed(items):
 out={}
 for value in items:
  if isinstance(value,dict) and 'declaration' in value:
   d=value['declaration']
   if 'impl' in d:
    i=d['impl'];key='impl:'+json.dumps([i['trait'],i['for'],i['generics'],i['is_synthetic']],sort_keys=True)
   else:key=str(value['name'])
  else:return None
  if key in out:
   old=out[key]['declaration'].get('impl')
   new=value['declaration'].get('impl')
   if old is None or new is None:return None
   if {k:v for k,v in old.items() if k!='items'} != {k:v for k,v in new.items() if k!='items'}:return None
   old['items'].extend(copy.deepcopy(new['items']))
  else:out[key]=copy.deepcopy(value)
 return out
def diff(a,b,path):
 if a==b:return []
 if isinstance(a,list) and isinstance(b,list) and path.rsplit('/',1)[-1] in ['items','impls','implementations']:
  x,y=keyed(a),keyed(b)
  if x is not None and y is not None:return diff(x,y,path)
 if isinstance(a,dict) and isinstance(b,dict):
  out=[]
  for k in sorted(a.keys()|b.keys()):
   if k not in a:out.append({'path':path+'/'+k,'change':'added'})
   elif k not in b:out.append({'path':path+'/'+k,'change':'removed'})
   else:out+=diff(a[k],b[k],path+'/'+k)
  return out
 return [{'path':path,'change':'changed','before':a,'after':b}]
result={}
for p in sorted((r/'compiler-64c5350').glob('*.api.json.gz')):
 a=json.loads(gzip.decompress((r/'compiler-c418e98'/p.name).read_bytes()))['exports']
 b=json.loads(gzip.decompress(p.read_bytes()))['exports']
 delta=diff(a,b,'exports')
 result[p.name.split('.api.')[0]]=delta
(r/'structural-diff.json').write_text(json.dumps(result,indent=2)+'\n')
for pkg,delta in result.items():
 print(pkg)
 for d in delta:
  if d['change']=='added':continue
  print(d['path'][:350],d['change'])
  if len(json.dumps(d))<2500: print(json.dumps(d))
